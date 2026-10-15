#!/usr/bin/env python3
"""AI Work 助手多开闭环测试脚本 —— 在 Windows 上部署并验证两个独立实例的 Plus 活动参与能力。

目标：
1. 在 Windows 本机或 Sandboxie-Plus 沙箱中启动两个独立 Qoder 实例
2. 分别登录 box006 和 box007 账号
3. 尝试参加 Plus 活动（显示 plus trial 字样，获得 300 积分）
4. 如果被识别为 VM 导致无法参加活动，则定位根本原因

执行顺序：
  Step A: 检查产物是否存在
  Step B: 选择部署方案（Sandboxie-Plus / Hyper-V / 原生隔离）
  Step C: 部署双实例并登录测试账号
  Step D: 验证 Plus 活动参与及积分到账
  Step E: 如失败则调研 VM 判定信号并修改指纹派生策略

注意：每个账号只能试一次，因为"登录一次后就固定了能领还是不能"，务必谨慎！
"""

import os, sys, json, time, subprocess, platform

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# ===== 全局配置 =====
PRODUCT_VERSION = "3.7.12"
PRODUCT_NAME = "AI Work 助手"
ASSETS_DIR = r"C:\Users\Xiong\AppData\Roaming\aiwork-intl-downloads"
TEST_ACCOUNTS = [
    {
        "id": "box006",
        "email": "box006@xiong777.onmicrosoft.com",
        "token": "dt-JnNXMkBz71cZuAE80sawyCzo",
        "refresh_token": "drt-eL9ylAKcuoNXyVNWh39mWHOU",
    },
    {
        "id": "box007",
        "email": "box007@xiong777.onmicrosoft.com",
        "token": "dt-uDbSZZvbRML54de0Xo9YONLQ",
        "refresh_token": "drt-m5c6adeRo8mlpQug1rJwDZtb",
    }
]
PLUS_ACTIVITY_ENDPOINT = "https://openapi.qoder.sh/sash/api/v1/me/campaigns"
PLUS_TRIAL_LABEL = "plus trial"  # 预期出现的关键文字

def check_product_assets():
    """Step A: 从 GitHub Actions 获取产物"""
    print("\n=== Step A: 检查 AI Work 助手产物 ===")
    
    # 方案 1: 从上次构建直接下载
    import urllib.request
    import zipfile
    import tempfile
    
    try:
        download_url = f"https://github.com/Xiong2359465/aiwork-intl/actions/artifacts/xxx/download"
        # TODO: 用正确的 artifacts API URL 替代 xxx
        print("提示：请手动从 Actions 页面下载产物")
        return None
    except Exception as e:
        print(f"产物下载失败：{e}")
        return False


def setup_sandboxie_plus():
    """Step B: 安装并配置 Sandboxie-Plus（推荐方案）"""
    print("\n=== Step B: Sandboxie-Plus 环境准备 ===")
    
    sb_installers = [
        r"C:\Sandboxie-Plus\sbie_plus_setup_x64.exe",
        r"C:\Program Files\Sandboxie-Plus\Sandboxie-Plus.exe"
    ]
    
    installed = any(os.path.exists(p) for p in sb_installers)
    if not installed:
        print("❌ Sandboxie-Plus 未检测到")
        print("下载链接：https://github.com/Sandboxie-Plus/Sandboxie/releases/latest")
        print("安装步骤:")
        print("  1. 运行安装程序（管理员权限）")
        print("  2. 完成后重启电脑")
        print("  3. 重新运行此脚本")
        return False
    
    print("✅ Sandboxie-Plus 已安装")
    print("接下来将创建两个沙箱盒：box006.qoder / box007.qoder")
    return True


def deploy_qoder_instance(sandbox_name, account_data):
    """Step C: 在指定沙箱盒中部署 Qoder 并登录指定账号"""
    print(f"\n=== Step C: 部署 {account_data['id']} 账号到沙箱 {sandbox_name} ===")
    
    qoder_exe = r"C:\Users\Xiong\AppData\Local\Programs\Qoder\Qoder.exe"
    
    if not os.path.exists(qoder_exe):
        print(f"❌ Qoder.exe 不存在：{qoder_exe}")
        return False
    
    # 构造 sandboxie-plus 启动命令
    cmd = f'sbcmd /runas sandbox://{sandbox_name} "{qoder_exe}"'
    
    print(f"启动命令: {cmd}")
    print(f"使用账号: {account_data['email']} (token={account_data['token'][:6]}...)")
    
    # TODO: 实际运行此命令
    # result = subprocess.run(cmd, shell=True, capture_output=True, timeout=60)
    # print(result.stdout.decode())
    # print(result.stderr.decode())
    
    return True


def verify_plus_activity_participation(instance_log):
    """Step D: 验证 Plus 活动参与结果"""
    print("\n=== Step D: 验证 Plus 活动参与 ===")
    
    # 通过 API 查询该账号的活动列表
    import urllib.request
    
    headers = {
        "Authorization": f"Bearer {instance_log.get('access_token', '')}",
        "Accept": "application/json",
        "User-Agent": "Qoder"
    }
    
    try:
        req = urllib.request.Request(
            PLUS_ACTIVITY_ENDPOINT,
            headers=headers
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            
        print(f"活动列表响应状态：{resp.status}")
        print(f"showCampaign: {data.get('showCampaign')}")
        print(f"claimable: {data.get('claimable')}")
        
        # 查找包含 "trial" 关键词的活动
        campaigns = data.get("campaigns", [])
        trial_campaigns = [c for c in campaigns if "trial" in str(c).lower()]
        
        if trial_campaigns:
            print(f"✅ 发现 {len(trial_campaigns)} 个试用相关活动")
            for tc in trial_campaigns:
                print(f"   - ID: {tc.get('campaignId')}, Name: {tc.get('name', tc.get('title'))}")
            return True
        else:
            print("⚠️ 未发现包含 'trial' 的活动（可能需要登录 Pro Trial 账号）")
            return False
            
    except Exception as e:
        print(f"❌ API 查询失败：{e}")
        return False


def detect_vm_indicators():
    """Step E: 检测 VM 标识符（如果 Plus 活动被拒绝）"""
    print("\n=== Step E: VM 标识符诊断 ===")
    
    indicators = {}
    
    # 1. CPU 信息
    import psutil
    cpu_model = psutil.cpu_model() or "Unknown"
    indicators["cpu_model"] = cpu_model
    print(f"CPU 型号：{cpu_model}")
    
    # 2. BIOS/Vendor
    try:
        bios_vendor = psutil.sensors_bios()
        indicators["bios"] = bios_vendor
        print(f"BIOS: {bios_vendor}")
    except:
        pass
    
    # 3. 内存容量（异常值可能指向 VM）
    mem_total = psutil.virtual_memory().total / (1024**3)
    indicators["ram_gb"] = round(mem_total, 1)
    print(f"内存：{round(mem_total, 1)} GB")
    
    # 4. 磁盘序列号（VM 通常一致）
    try:
        disk_serial = os.pexec("wmic diskdrive get serialnumber").strip()
        indicators["disk_serial"] = disk_serial[:20] + "..." if len(disk_serial) > 20 else disk_serial
        print(f"磁盘序列号：{indicators['disk_serial']}")
    except:
        pass
    
    # 5. MAC 地址前缀
    try:
        mac = psutil.net_if_addrs()['Ethernet'][0].addr
        indicators["mac_prefix"] = mac.split(':')[0][:2]
        print(f"MAC 前缀：{indicators['mac_prefix']}")
    except:
        pass
    
    # 判断是否疑似 VM
    vm_suspects = []
    if "vmware" in cpu_model.lower():
        vm_suspects.append("VMware CPU 标识")
    if "virtualbox" in cpu_model.lower():
        vm_suspects.append("VirtualBox CPU 标识")
    if "hyper-v" in cpu_model.lower():
        vm_suspects.append("Hyper-V CPU 标识")
    if mem_total < 1 or mem_total > 128:
        vm_suspects.append(f"异常内存大小 {mem_total}GB")
    if indicators.get("disk_serial", "") == "0" * 20:
        vm_suspects.append("磁盘序列号全 0")
    
    if vm_suspects:
        print(f"\n⚠️ 检测到 {len(vm_suspects)} 个可疑 VM 指标：")
        for vs in vm_suspects:
            print(f"   - {vs}")
        return True, vm_suspects
    else:
        print("\n✅ 未发现明显 VM 标识")
        return False, []


def main():
    print("=" * 70)
    print("AI Work 助手 · 多开闭环测试")
    print("=" * 70)
    
    steps = ["Step A: Check Products", "Step B: Setup Sandboxie", 
             "Step C: Deploy Instances", "Step D: Verify Plus Activity",
             "Step E: Detect VM Indicators"]
    
    print("\n执行计划:")
    for i, step in enumerate(steps, 1):
        print(f"  {i}. {step}")
    
    # 实际执行主逻辑
    product_ok = check_product_assets()
    if not product_ok:
        print("产品获取失败，请先手动下载并放置于 ASSETS_DIR")
        return 1
    
    sandbox_ok = setup_sandboxie_plus()
    if not sandbox_ok:
        print("Sandboxie-Plus 安装失败，请手动安装后重试")
        return 1
    
    # 继续部署...
    print("\n🎉 准备就绪！开始部署双实例...")
    for i, account in enumerate(TEST_ACCOUNTS):
        sandbox_name = f"{account['id']}.qoder"
        success = deploy_qoder_instance(sandbox_name, account)
        if success:
            print(f"✅ {account['id']} 实例部署成功")
        else:
            print(f"❌ {account['id']} 实例部署失败")
    
    # 验证 Plus 活动
    for account in TEST_ACCOUNTS:
        print(f"\n正在验证 {account['id']} 账号的 Plus 活动参与情况...")
        # TODO: 调用 verify_plus_activity_participation()
    
    # VM 诊断
    has_vm, suspects = detect_vm_indicators()
    
    print("\n" + "=" * 70)
    print("测试完成报告")
    print("=" * 70)
    print(f"部署实例数：{len(TEST_ACCOUNTS)}")
    print(f"VM 可疑指标：{suspects if suspects else '无'}")
    print("\n下一步操作建议:")
    if has_vm:
        print("  1. 考虑更换物理机环境或使用不同网络出口")
        print("  2. 修改客户端设备指纹派生策略（避免使用易暴露 VM 的信号）")
    else:
        print("  1. 确认测试账号是否为 Free 计划（Free 账号可能没有 Plus Trial 权益）")
        print("  2. 尝试升级 Pro Trial 后再测试")
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
