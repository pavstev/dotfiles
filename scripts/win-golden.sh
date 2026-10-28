#!/usr/bin/env bash
# ============================================================================
#  scripts/win-golden.sh: build the UTM VM "win11-golden" that scripts/os-check.sh clones.
#  macOS only (Apple silicon). Run it once. os-check never starts or changes the golden VM.
#
#    bash scripts/win-golden.sh                 # build, check, shut down
#    bash scripts/win-golden.sh --rebuild       # delete an old win11-golden first
#
#  Needs (it checks and says what is missing):
#    - UTM:               brew install --cask utm
#    - the Windows ISO:   Windows 11 Arm64 from microsoft.com/software-download/windows11arm64
#                         saved as ~/VMs/iso/Win11_arm64_en-us.iso (the link on that page lasts
#                         a day, so this script cannot fetch it for you)
#    - the guest tools:   fetched by this script (utm-guest-tools-latest.iso, about 120 MB)
#    - an SSH key:        WIN_KEY, default ~/.ssh/os-check_ed25519 (made here if missing)
#
#  What it builds: a QEMU VM (UEFI, 4 cores, 6 GB RAM, 64 GB disk), then one unattended install:
#  a local admin account (no Microsoft account), the UTM guest tools, OpenSSH Server with your
#  public key, Developer Mode, Git for Windows and PowerShell 7 (WinGet, agreements accepted).
#  The answer file goes in ~/VMs; nothing personal is read from or written to the repo.
#  Env: WIN_VM (name), WIN_DIR (default ~/VMs), WIN_ISO, WIN_KEY, WIN_USER (default dots).
# ============================================================================
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=setup-lib.sh
. "$REPO/scripts/setup-lib.sh"
is_macos || { echo "win-golden.sh runs on macOS (UTM)"; exit 2; }

VM="${WIN_VM:-win11-golden}"
DIR="${WIN_DIR:-$HOME/VMs}"
ISO="${WIN_ISO:-$DIR/iso/Win11_arm64_en-us.iso}"
TOOLS="$DIR/iso/utm-guest-tools.iso"
KEY="${WIN_KEY:-$HOME/.ssh/os-check_ed25519}"
WUSER="${WIN_USER:-dots}"
REBUILD=0
[[ "${1:-}" == --rebuild ]] && REBUILD=1
UTMCTL="$(command -v utmctl || echo /Applications/UTM.app/Contents/MacOS/utmctl)"
SSHO=(-i "$KEY" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no
      -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5)

say() { printf '»» %s\n' "$*"; }
die() { printf 'FAIL  %s\n' "$*" >&2; exit 1; }

# UTM shows a "What's New" box on its first start, and no script can start a VM until it is closed.
defaults write com.utmapp.UTM ShowReleaseNotes -bool false 2>/dev/null || true

# ---- 0. checks ----------------------------------------------------------------
[[ -x "$UTMCTL" ]] || die "UTM is missing. Run: brew install --cask utm"
[[ -f "$ISO" ]] || die "no Windows ISO at $ISO (get it from microsoft.com/software-download/windows11arm64)"
if "$UTMCTL" list 2>/dev/null | grep -qw "$VM"; then
  ((REBUILD)) || die "$VM already exists. Use --rebuild to delete it and build again"
  "$UTMCTL" stop "$VM" --kill >/dev/null 2>&1 || true
  "$UTMCTL" delete "$VM" >/dev/null
  # utmctl can exit 0 while UTM still lists the VM: wait, then stop with the fix.
  for _ in $(seq 15); do "$UTMCTL" list 2>/dev/null | grep -qw "$VM" || break; sleep 2; done
  "$UTMCTL" list 2>/dev/null | grep -qw "$VM" && die "UTM still lists $VM. Quit and reopen UTM, then run this again"
fi
mkdir -p "$DIR/iso"
if [[ ! -f "$KEY" ]]; then
  say "making the SSH key $KEY (no passphrase, only for the check VMs)"
  ssh-keygen -q -t ed25519 -N '' -C os-check -f "$KEY"
fi
if [[ ! -f "$TOOLS" ]]; then
  say "downloading the UTM guest tools"
  curl -fsSL -o "$TOOLS.part" https://getutm.app/downloads/utm-guest-tools-latest.iso && mv "$TOOLS.part" "$TOOLS"
fi

# ---- 1. the seed ISO: answer file + first-logon script + the guest tools -------------
PASS="$(openssl rand -hex 12)"
(umask 077; printf 'user=%s\npassword=%s\n' "$WUSER" "$PASS" >"$DIR/$VM.cred")  # scan-allowlist: the VM's own random login, kept outside the repo
W="$(mktemp -d)"; trap 'hdiutil detach "$W.mnt" -quiet 2>/dev/null || true; rm -rf "$W" "$W.mnt"' EXIT
mkdir -p "$W.mnt" "$W/seed"
hdiutil attach -readonly -nobrowse -mountpoint "$W.mnt" "$TOOLS" >/dev/null
cp -R "$W.mnt"/. "$W/seed/"            # the drivers and the installer (the original answer file goes)
hdiutil detach "$W.mnt" -quiet
chmod -R u+w "$W/seed"
rm -f "$W/seed/Autounattend.xml"
cp "$KEY.pub" "$W/seed/authorized_key.pub"

CA='processorArchitecture="arm64" publicKeyToken="31bf3856ad364e35" language="neutral" versionScope="nonSxS"'  # scan-allowlist: Microsoft's public component token, not a secret
cat >"$W/seed/Autounattend.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<unattend xmlns="urn:schemas-microsoft-com:unattend" xmlns:wcm="http://schemas.microsoft.com/WMIConfig/2002/State">
  <settings pass="windowsPE">
    <component name="Microsoft-Windows-International-Core-WinPE" $CA>
      <SetupUILanguage><UILanguage>en-US</UILanguage></SetupUILanguage>
      <InputLocale>en-US</InputLocale><SystemLocale>en-US</SystemLocale><UILanguage>en-US</UILanguage><UserLocale>en-US</UserLocale>
    </component>
    <component name="Microsoft-Windows-Setup" $CA>
      <RunSynchronous>
        <RunSynchronousCommand wcm:action="add"><Order>1</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassTPMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>2</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassSecureBootCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>3</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassRAMCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
        <RunSynchronousCommand wcm:action="add"><Order>4</Order><Path>reg add HKLM\System\Setup\LabConfig /v BypassCPUCheck /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
      </RunSynchronous>
      <DiskConfiguration>
        <Disk wcm:action="add">
          <DiskID>0</DiskID><WillWipeDisk>true</WillWipeDisk>
          <CreatePartitions>
            <CreatePartition wcm:action="add"><Order>1</Order><Type>EFI</Type><Size>300</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>2</Order><Type>MSR</Type><Size>16</Size></CreatePartition>
            <CreatePartition wcm:action="add"><Order>3</Order><Type>Primary</Type><Extend>true</Extend></CreatePartition>
          </CreatePartitions>
          <ModifyPartitions>
            <ModifyPartition wcm:action="add"><Order>1</Order><PartitionID>1</PartitionID><Format>FAT32</Format><Label>System</Label></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>2</Order><PartitionID>2</PartitionID></ModifyPartition>
            <ModifyPartition wcm:action="add"><Order>3</Order><PartitionID>3</PartitionID><Format>NTFS</Format><Label>Windows</Label><Letter>C</Letter></ModifyPartition>
          </ModifyPartitions>
        </Disk>
      </DiskConfiguration>
      <ImageInstall>
        <OSImage>
          <InstallTo><DiskID>0</DiskID><PartitionID>3</PartitionID></InstallTo>
          <InstallFrom><MetaData wcm:action="add"><Key>/IMAGE/NAME</Key><Value>Windows 11 Pro</Value></MetaData></InstallFrom>
        </OSImage>
      </ImageInstall>
      <UserData>
        <AcceptEula>true</AcceptEula>
        <ProductKey><Key>W269N-WFGWX-YVC9B-4J6C9-T83GX</Key><WillShowUI>OnError</WillShowUI></ProductKey>
      </UserData>
    </component>
  </settings>
  <settings pass="specialize">
    <component name="Microsoft-Windows-Shell-Setup" $CA>
      <ComputerName>WIN11-CHECK</ComputerName>
      <TimeZone>UTC</TimeZone>
    </component>
    <component name="Microsoft-Windows-Deployment" $CA>
      <RunSynchronous>
        <RunSynchronousCommand wcm:action="add"><Order>1</Order><Path>reg add HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE /v BypassNRO /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>
      </RunSynchronous>
    </component>
  </settings>
  <settings pass="oobeSystem">
    <component name="Microsoft-Windows-International-Core" $CA>
      <InputLocale>en-US</InputLocale><SystemLocale>en-US</SystemLocale><UILanguage>en-US</UILanguage><UserLocale>en-US</UserLocale>
    </component>
    <component name="Microsoft-Windows-Shell-Setup" $CA>
      <OOBE>
        <HideEULAPage>true</HideEULAPage>
        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>
        <HideLocalAccountScreen>true</HideLocalAccountScreen>
        <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>
        <ProtectYourPC>3</ProtectYourPC>
        <NetworkLocation>Work</NetworkLocation>
      </OOBE>
      <UserAccounts>
        <LocalAccounts>
          <LocalAccount wcm:action="add">
            <Name>$WUSER</Name><Group>Administrators</Group><DisplayName>$WUSER</DisplayName>
            <Password><Value>$PASS</Value><PlainText>true</PlainText></Password>
          </LocalAccount>
        </LocalAccounts>
      </UserAccounts>
      <AutoLogon>
        <Enabled>true</Enabled><LogonCount>5</LogonCount><Username>$WUSER</Username>
        <Password><Value>$PASS</Value><PlainText>true</PlainText></Password>
      </AutoLogon>
      <FirstLogonCommands>
        <SynchronousCommand wcm:action="add">
          <Order>1</Order>
          <Description>golden setup</Description>
          <CommandLine>cmd /c "for %d in (D E F G H I J) do if exist %d:\setup.ps1 powershell -NoProfile -ExecutionPolicy Bypass -File %d:\setup.ps1"</CommandLine>
        </SynchronousCommand>
      </FirstLogonCommands>
    </component>
  </settings>
</unattend>
EOF

cat >"$W/seed/setup.ps1" <<'EOF'
# First logon, as the new admin. Logs to C:\Windows\Temp\golden-setup.log. Writes C:\golden-ready.txt at the end.
$ErrorActionPreference = 'Continue'
Start-Transcript -Path C:\Windows\Temp\golden-setup.log -Force | Out-Null
$seed = Split-Path -Parent $MyInvocation.MyCommand.Path

# Developer Mode: native symlinks without elevation (what ./dotfiles needs).
reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock" /t REG_DWORD /f /v AllowDevelopmentWithoutDevLicense /d 1
powercfg -h off

# UTM guest tools (drivers, SPICE agent, QEMU guest agent), silent.
$tools = Get-ChildItem "$seed\utm-guest-tools-*.exe" | Select-Object -First 1
Start-Process -FilePath $tools.FullName -ArgumentList '/S' -Wait

# Network (the virtio driver comes with the tools).
for ($i = 0; $i -lt 60; $i++) { if (Test-Connection -Quiet -Count 1 aka.ms) { break }; Start-Sleep 5 }

# OpenSSH Server, your public key for administrators, service automatic, firewall rule.
for ($i = 0; $i -lt 10; $i++) {
  Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
  if (Get-Service sshd -ErrorAction SilentlyContinue) { break }; Start-Sleep 20
}
Set-Service sshd -StartupType Automatic
Start-Service sshd
if (-not (Get-NetFirewallRule -Name 'sshd' -ErrorAction SilentlyContinue)) {
  New-NetFirewallRule -Name sshd -DisplayName 'OpenSSH Server (sshd)' -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
}
$ak = 'C:\ProgramData\ssh\administrators_authorized_keys'
Copy-Item "$seed\authorized_key.pub" $ak -Force
icacls.exe $ak /inheritance:r /grant 'SYSTEM:(F)' /grant 'BUILTIN\Administrators:(F)'

# WinGet: register it (a fresh image has not yet), then retry until it works.
Add-AppxPackage -RegisterByFamilyName -MainPackage Microsoft.DesktopAppInstaller_8wekyb3d8bbwe -ErrorAction SilentlyContinue
foreach ($id in 'Git.Git', 'Microsoft.PowerShell') {
  for ($i = 0; $i -lt 12; $i++) {
    winget install --id $id -e --silent --accept-package-agreements --accept-source-agreements
    if ($LASTEXITCODE -eq 0 -or $LASTEXITCODE -eq -1978335189) { break }   # ok, or already installed
    Start-Sleep 30
  }
}

# sshd reads PATH once at start: restart it so new sessions see git and pwsh.
Restart-Service sshd
'ok' | Out-File C:\golden-ready.txt -Encoding ascii
Stop-Transcript | Out-Null
EOF

say "building the seed ISO"
mkdir -p "$DIR/seed"
SEED="$DIR/seed/$VM-seed.iso"
rm -f "$SEED" "${SEED%.iso}.cdr"
hdiutil makehybrid -quiet -iso -joliet -default-volume-name WINSEED -o "${SEED%.iso}" "$W/seed"
[[ -f "$SEED" ]] || mv "${SEED%.iso}.iso" "$SEED" 2>/dev/null || true
[[ -f "$SEED" ]] || die "no seed ISO made"

# ---- 2. the VM (UTM AppleScript) ----------------------------------------------------
say "making the VM $VM"
osascript - "$VM" "$ISO" "$SEED" >/dev/null <<'EOF'
on run argv
  set winIso to POSIX file (item 2 of argv)
  set seedIso to POSIX file (item 3 of argv)
  tell application "UTM"
    make new virtual machine with properties {backend:qemu, configuration:{name:(item 1 of argv), architecture:"aarch64", machine:"virt", memory:6144, cpu cores:4, uefi:true, hypervisor:true, drives:{{removable:true, interface:USB, source:seedIso}, {removable:true, interface:USB, source:winIso}, {interface:NVMe, guest size:65536}}, network interfaces:{{hardware:"virtio-net-pci", mode:shared}}, displays:{{hardware:"virtio-ramfb"}}}}
  end tell
end run
EOF
"$UTMCTL" list | grep -qw "$VM" || die "UTM did not make $VM"
# TPM 2.0 has no AppleScript property: set it in the VM's config.plist, then reload it.
CFG="$HOME/Library/Containers/com.utmapp.UTM/Data/Documents/$VM.utm/config.plist"
plutil -replace QEMU.TPMDevice -bool true "$CFG" || die "couldn't turn on the TPM in $CFG"
osascript -e "tell application \"UTM\" to reload configuration (virtual machine named \"$VM\")" >/dev/null

# ---- 3. install ---------------------------------------------------------------------
say "starting the VM and pressing a key for the installer's boot prompt"
"$UTMCTL" start "$VM"
# utmctl start can time out (OSStatus -1712) and still exit 0, with the VM left stopped.
for _ in $(seq 15); do "$UTMCTL" status "$VM" 2>/dev/null | grep -qi started && break; sleep 2; done
"$UTMCTL" status "$VM" 2>/dev/null | grep -qi started \
  || die "UTM did not start $VM. Quit and reopen UTM, then run: bash scripts/win-golden.sh --rebuild"
# The Windows ISO asks "press any key to boot from CD" for a few seconds. Send an "x" every 2 seconds
# for 60 seconds (UTM's own key input, so no extra permission is needed). Not a space or Enter: once
# Setup runs, its focused Cancel button would take them.
osascript -e "tell application \"UTM\"" -e "set vm to virtual machine named \"$VM\"" \
  -e 'repeat 30 times' -e 'input keystroke vm text "x"' -e 'delay 2' -e 'end repeat' -e 'end tell' >/dev/null 2>&1 || true
say "waiting for the install, up to 60 minutes (SSH answers and C:\\golden-ready.txt exists)"
ip="" end=$((SECONDS + 3600))
while ((SECONDS < end)); do
  ip="$("$UTMCTL" ip-address "$VM" 2>/dev/null | grep -Eo '^[0-9]+(\.[0-9]+){3}' | head -n1 || true)"
  if [[ -n "$ip" ]] && ssh "${SSHO[@]}" "$WUSER@$ip" 'if exist C:\golden-ready.txt echo ready' 2>/dev/null | grep -q ready; then break; fi
  ip=""; sleep 20
done
[[ -n "$ip" ]] || die "the install did not finish in time. Look at the VM window in UTM"

# ---- 4. check, shut down ------------------------------------------------------------
say "checking the VM at $ip"
sshv() { ssh "${SSHO[@]}" "$WUSER@$ip" "$@"; }
sshv 'git --version' || die "git is missing in the VM"
sshv 'pwsh -NoProfile -Command $PSVersionTable.PSVersion.ToString()' || die "pwsh is missing in the VM"
sshv 'reg query HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock /v AllowDevelopmentWithoutDevLicense' | grep -q 0x1 \
  || die "Developer Mode is off in the VM"
say "shutting the VM down"
sshv 'shutdown /s /t 5' || true
for _ in $(seq 60); do
  "$UTMCTL" status "$VM" | grep -qi stopped && break
  sleep 5
done
"$UTMCTL" status "$VM" | grep -qi stopped || "$UTMCTL" stop "$VM" --force
say "done. $VM is shut down. os-check clones it; never start it again."
