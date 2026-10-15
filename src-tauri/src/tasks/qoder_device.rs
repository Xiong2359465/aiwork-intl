//! Qoder 设备指纹域（F-80 M3，§5.10 多账号并发；v1.2 用户决策：伪造是正式需求，
//! 以「每账号稳定绑定」而非随机轮换作为风险控制手段）。
//!
//! 移植自已验证的社区实现模型（docs/completed/Qoder外部项目调研归档.md）：
//! - QoderGateway（§2.3）：machine_id 账号级持久稳定入库；machine_token/machine_type
//!   每会话现场随机；服务端无强绑定校验
//! - QoderPatcher（§1.3/§6.1）：machine_id = 32 位小写 hex 无分隔符；device_id/umid
//!   = uuid4 标准格式。原实现为 md5(rand)，按归档 §6.1 建议改 uuid4 种子增熵——
//!   本实现用 sha256(uuid4)[..32] 产同格式 hex，不引入 md5 crate
//!
//! 注入优先级（tasks::qoder_common::effective_creds 合并）：
//! ① token store 有真实捕获 machine_id（client/mitm/cli 来源）→ 透传真实值；
//! ② 否则注入 device_profile.machine_id + 现场随机 machine_token；
//! ③ 全无 → 不带设备头（build_auth_headers 缺失即不带）。
//!
//! §5.10.2 本地存储覆写已实现：switcher/machine.rs apply_qoder_fingerprint 在切换/恢复
//! 成功后覆写本地 machineid 文件 + storage.json 遥测三键 + state.vscdb 服务端机器码
//! （挂点 switcher::apply_fingerprint_override）；本模块仍负责 API 请求侧指纹。

use sha2::{Digest, Sha256};

use crate::state::AppState;

/// 每账号稳定设备指纹（§5.10.1；入库 qoder_pool 账号记录，一次生成永不轮换）
#[derive(serde::Serialize, serde::Deserialize, Clone, Default, Debug)]
pub struct QoderDeviceProfile {
    /// 32 位小写 hex（Cosy-MachineId 注入源）
    #[serde(default)]
    pub machine_id: String,
    /// uuid4 标准格式（客户端设备 id 形态；ms_deviceid 同源，§1.4）
    #[serde(default)]
    pub device_id: String,
    /// uuid4 标准格式（umid 仅 macOS 强制，Windows 预留对齐字段）
    #[serde(default)]
    pub umid: String,
}

impl QoderDeviceProfile {
    /// 生成全新指纹（仅在账号入池/回填且无既有指纹时调用一次，此后稳定不变）
    pub fn generate() -> Self {
        Self {
            machine_id: machine_id_generate(),
            device_id: uuid::Uuid::new_v4().to_string(),
            umid: uuid::Uuid::new_v4().to_string(),
        }
    }
}

// ── 国际版：账号绑定的稳定设备标识派生 ──────────────────────────────────────

/// 派生出的设备标识四键，形态对齐国际版客户端实测头：
/// machine_id 32 hex / machine_code 18 hex / machine_type 17 hex /
/// machine_token 43 位 base64url。
#[derive(Clone, Default, Debug)]
pub struct DerivedDeviceIdentity {
    pub machine_id: String,
    pub machine_code: String,
    pub machine_type: String,
    pub machine_token: String,
}

/// 派生种子：优先账号 uid，uid 未知时回退凭证——同一凭证跨调用结果恒定，
/// 不随进程重启漂移（漂移即换设备，等价触发风控）
pub fn fingerprint_seed(uid: &str, credential: &str) -> String {
    if !uid.is_empty() {
        uid.to_string()
    } else {
        format!("cred:{credential}")
    }
}

fn md5_hex(input: &str) -> String {
    use md5::{Digest as _, Md5};
    let mut h = Md5::new();
    h.update(input.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 按账号派生设备标识。
///
/// `device_profile.machine_id`（入池生成一次、永不轮换）在此充当**该账号自持的
/// 私有盐**，与 uid/凭证一起参与单向派生。多开意义上的两个不变量：
/// ① 同一账号长期表现为同一台虚拟设备（机器码不漂移）；
/// ② 不同账号彼此落在不同桶里，跨账号无共同机器标识（防关联判定）。
/// 盐只存在本地账号记录内，故外部即便知道 uid 也推不出指纹——纯 uid 派生
/// 正是这个缺陷（上游一旦识别派生模式可全局拉黑）。
///
/// 规则与 qoder2api `internal/cosy/fingerprint.go` 同构（machine / machinetype /
/// machinetoken 三标签 + 盐混入），并额外派生 machinecode 以补齐国际版要求的
/// 全部设备头（缺任一 cosy-machine* 头，服务端不下发签到活动）。
pub fn derive_device_identity(
    profile: &QoderDeviceProfile,
    uid: &str,
    credential: &str,
) -> DerivedDeviceIdentity {
    let base = fingerprint_seed(uid, credential);
    let seed = if profile.machine_id.is_empty() {
        base
    } else {
        format!("{base}|salt:{}", profile.machine_id)
    };
    let mut sha = <sha2::Sha512 as sha2::Digest>::new();
    sha.update(format!("machinetoken:{seed}").as_bytes());
    let token_enc = {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha.finalize())
    };
    DerivedDeviceIdentity {
        machine_id: md5_hex(&format!("machine:{seed}")),
        machine_code: md5_hex(&format!("machinecode:{seed}"))[..18].to_string(),
        machine_type: md5_hex(&format!("machinetype:{seed}"))[..17].to_string(),
        machine_token: token_enc[..43.min(token_enc.len())].to_string(),
    }
}

/// machine_id 生成：sha256(uuid4)[..32] —— 32 位小写 hex，与 QoderPatcher 的
/// md5(随机数) 同格式且熵更高（归档 §6.1 翻译建议）
pub fn machine_id_generate() -> String {
    let seed = uuid::Uuid::new_v4().to_string();
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    hex[..32].to_string()
}

/// Cosy-MachineToken 现场随机值（QoderGateway 取值模型：每会话随机，非持久绑定）
pub fn random_machine_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 指纹合并（纯函数便于测试；effective_creds 唯一调用）：
/// 真实捕获优先透传；缺失时注入账号绑定 machine_id + 现场随机 machine_token。
/// profile 缺失或 machine_id 为空时不注入（请求头自然不带设备头）。
pub fn merge_device_profile(creds: &mut crate::tasks::qoder_common::QoderCreds, profile: Option<&QoderDeviceProfile>) {
    if !creds.machine_id.is_empty() {
        return; // 真实捕获（client/mitm/cli 来源）优先
    }
    if let Some(p) = profile.filter(|p| !p.machine_id.is_empty()) {
        creds.machine_id = p.machine_id.clone();
        creds.machine_token = random_machine_token();
    }
}

/// 账号池惰性回填（幂等）：为缺指纹的存量账号生成 profile；已有不覆盖。
/// 直操 qoder_pool 原始 JSON（保留未知字段），所有签到/列表路径共用。
pub fn ensure_pool_profiles(state: &AppState) -> Result<(), String> {
    // I09：直操原始 JSON 保留未知字段（不可走 with_pool_mut），持池锁防并发整池覆盖丢写
    let _guard = state
        .qoder_pool_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let db = crate::store::db(&state.data_dir);
    let mut pool = crate::store::docs::qoder_pool_load(&db);
    let Some(accounts) = pool.get_mut("accounts").and_then(|a| a.as_array_mut()) else {
        return Ok(());
    };
    let mut changed = 0usize;
    for a in accounts.iter_mut() {
        let has = a
            .get("device_profile")
            .and_then(|p| p.get("machine_id"))
            .and_then(|m| m.as_str())
            .is_some_and(|s| !s.is_empty());
        if !has {
            if let Some(o) = a.as_object_mut() {
                let profile = serde_json::to_value(QoderDeviceProfile::generate())
                    .map_err(|e| e.to_string())?;
                o.insert("device_profile".into(), profile);
                changed += 1;
            }
        }
    }
    if changed > 0 {
        crate::store::docs::qoder_pool_save(&db, &pool)?;
        crate::fs_utils::app_log(
            &state.data_dir,
            &format!("Qoder 设备指纹回填完成（本轮补齐 {changed} 个账号）"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 格式基线：machine_id 32 位小写 hex；device_id/umid 为 uuid4 标准形态
    #[test]
    fn generated_profile_formats() {
        let p = QoderDeviceProfile::generate();
        assert_eq!(p.machine_id.len(), 32, "32 位 hex");
        assert!(p.machine_id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(p.machine_id.chars().all(|c| !c.is_ascii_uppercase()), "小写");
        for u in [&p.device_id, &p.umid] {
            let parts: Vec<&str> = u.split('-').collect();
            assert_eq!(parts.iter().map(|s| s.len()).collect::<Vec<_>>(), vec![8, 4, 4, 4, 12]);
            assert!(u.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        }
    }

    /// 随机性：两次生成互异（防写死）；machine_token 现场随机
    #[test]
    fn generation_is_random() {
        let a = QoderDeviceProfile::generate();
        let b = QoderDeviceProfile::generate();
        assert_ne!(a.machine_id, b.machine_id);
        assert_ne!(a.device_id, b.device_id);
        assert_ne!(random_machine_token(), random_machine_token());
    }

    /// 注入优先级：真实捕获 wins（profile 不覆盖已有 machine_id）
    #[test]
    fn merge_prefers_captured_machine_id() {
        let mut creds = crate::tasks::qoder_common::QoderCreds {
            machine_id: "captured-real".into(),
            machine_token: "captured-token".into(),
            ..Default::default()
        };
        let p = QoderDeviceProfile::generate();
        merge_device_profile(&mut creds, Some(&p));
        assert_eq!(creds.machine_id, "captured-real");
        assert_eq!(creds.machine_token, "captured-token");
    }

    /// 注入：缺失时注入 profile.machine_id + 现场随机 machine_token
    #[test]
    fn merge_injects_profile_with_fresh_token() {
        let mut creds = crate::tasks::qoder_common::QoderCreds::default();
        let p = QoderDeviceProfile::generate();
        merge_device_profile(&mut creds, Some(&p));
        assert_eq!(creds.machine_id, p.machine_id);
        assert!(!creds.machine_token.is_empty());
        // 每次调用现场随机：同 profile 两次合并 token 互异（QoderGateway 模型）
        let mut creds2 = crate::tasks::qoder_common::QoderCreds::default();
        merge_device_profile(&mut creds2, Some(&p));
        assert_ne!(creds.machine_token, creds2.machine_token);
    }

    /// 全无：profile 缺失/空 machine_id 时不注入（请求头自然不带设备头）
    #[test]
    fn merge_without_profile_is_noop() {
        let mut creds = crate::tasks::qoder_common::QoderCreds::default();
        merge_device_profile(&mut creds, None);
        assert!(creds.machine_id.is_empty());
        assert!(creds.machine_token.is_empty());
        let empty = QoderDeviceProfile::default();
        merge_device_profile(&mut creds, Some(&empty));
        assert!(creds.machine_id.is_empty());
    }
}

#[cfg(test)]
mod derive_identity_tests {
    use super::*;

    fn profile_with(mid: &str) -> QoderDeviceProfile {
        QoderDeviceProfile {
            machine_id: mid.to_string(),
            ..Default::default()
        }
    }

    /// 无盐（profile.machine_id 为空）时与 hub / qoder2api 的派生逐字节同构：
    /// md5("machine:{uid}") / md5("machinecode:{uid}")[:18] /
    /// md5("machinetype:{uid}")[:17] / b64url_nopad(sha512("machinetoken:{uid}"))[:43]。
    /// 期望值由独立实现（Python hashlib + base64）离线算出并硬编码于此。
    #[test]
    fn derive_matches_reference_vectors_without_salt() {
        let d = derive_device_identity(&profile_with(""), "u-test", "dt-irrelevant");
        assert_eq!(d.machine_id, "ca0bfb324ea94332ad402d6311455a3b");
        assert_eq!(d.machine_code, "7bb731df9a7417fcde");
        assert_eq!(d.machine_type, "73bcf1fc641e54a50");
        assert_eq!(d.machine_token, "8H5OX8u_I9jcPcyUWJLzxhbhqf7cS3nYofUl0Um4E3p");
    }

    /// 形态红线：32/18/17 hex 小写 + 43 位 base64url，与真机抓包同形
    /// （长度或字符集不对，一眼就是自造设备）
    #[test]
    fn derived_shape_looks_like_real_client() {
        let p = QoderDeviceProfile::generate();
        let d = derive_device_identity(&p, "u-any", "dt-x");
        assert_eq!(d.machine_id.len(), 32);
        assert_eq!(d.machine_code.len(), 18);
        assert_eq!(d.machine_type.len(), 17);
        assert_eq!(d.machine_token.len(), 43);
        for s in [&d.machine_id, &d.machine_code, &d.machine_type] {
            assert!(
                s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "非小写 hex: {s}"
            );
        }
        assert!(d
            .machine_token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    /// 同账号 + 同 profile：跨调用恒定（换值 = 换设备 = 触发风控）
    #[test]
    fn same_account_is_stable_across_calls() {
        let p = profile_with("b".repeat(32).as_str());
        let a = derive_device_identity(&p, "uid-1", "dt-1");
        let b = derive_device_identity(&p, "uid-1", "dt-1");
        assert_eq!(a.machine_id, b.machine_id);
        assert_eq!(a.machine_code, b.machine_code);
        assert_eq!(a.machine_type, b.machine_type);
        assert_eq!(a.machine_token, b.machine_token);
    }

    /// 多开核心不变量：uid 不同 或 账号私有盐不同 → 四键全不同，
    /// 同机多账号在外部分属不同设备，无共享标识可供关联
    #[test]
    fn accounts_are_mutually_distinct() {
        let p1 = profile_with("a".repeat(32).as_str());
        let p2 = profile_with("b".repeat(32).as_str());
        let d1 = derive_device_identity(&p1, "uid-1", "dt-1");
        let d2 = derive_device_identity(&p1, "uid-2", "dt-2");
        let d3 = derive_device_identity(&p2, "uid-1", "dt-1");
        assert_ne!(d1.machine_id, d2.machine_id);
        assert_ne!(d1.machine_id, d3.machine_id);
        assert_ne!(d1.machine_token, d3.machine_token);
    }

    /// uid 未知时以凭证为种子：同一凭证重导入得到同一指纹（不因导入次数漂移），
    /// 不同凭证彼此不同
    #[test]
    fn credential_fallback_is_deterministic() {
        let p = profile_with("c".repeat(32).as_str());
        let a = derive_device_identity(&p, "", "dt-same");
        let b = derive_device_identity(&p, "", "dt-same");
        let c = derive_device_identity(&p, "", "dt-other");
        assert_eq!(a.machine_id, b.machine_id);
        assert_ne!(a.machine_id, c.machine_id);
    }

    /// 带账号私有盐（真实入池形态）时不再等于纯 uid 派生值——防「知道 uid 即可
    /// 反推指纹」，也与其它部署的派生空间隔离
    #[test]
    fn per_account_salt_moves_identity_off_plain_uid_space() {
        let plain = derive_device_identity(&profile_with(""), "u-test", "dt-x");
        let salted = derive_device_identity(&profile_with(&"a".repeat(32)), "u-test", "dt-x");
        assert_eq!(salted.machine_id, "b998bd1e9f03aa271fb52bcea375f7ea");
        assert_ne!(plain.machine_id, salted.machine_id);
    }
}
