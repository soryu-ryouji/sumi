# 发包（桌面应用）：构建 sumi 桌面应用的 Windows 分发包并归置到输出目录（默认仓库根目录的 out/）。
# 产物为免安装 zip（sumi-windows-x64.zip，解压即用）。
#
# 用法: ./tools/build-app.ps1 [-Path <输出目录>] [-Unpacked]
#   -Unpacked：只产出未打包目录（win-unpacked），install.ps1 的复用入口
# 前置: 最新 Node.js 与 Rust 工具链（https://rustup.rs/）

$ErrorActionPreference = 'Stop'

$Path = ''
$Unpacked = $false
$argsList = @($args)
$i = 0
while ($i -lt $argsList.Count) {
    $a = [string]$argsList[$i]
    if ($a -match '^(-Path|--path)$') {
        if ($i + 1 -ge $argsList.Count) { throw "$a 需要目录参数" }
        $Path = [string]$argsList[$i + 1]; $i += 2
    } elseif ($a -match '^--?path=(.+)$') {
        $Path = $Matches[1]; $i++
    } elseif ($a -match '^(-Unpacked|--unpacked)$') {
        $Unpacked = $true; $i++
    } else {
        throw "未知参数: $a（用法: ./tools/build-app.ps1 [-Path <输出目录>] [-Unpacked]）"
    }
}

foreach ($tool in @('node', 'npm', 'cargo')) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "未找到 $tool，请先安装最新的 Node.js 与 Rust 工具链（https://rustup.rs/）"
    }
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
$AppDir = Join-Path $RepoRoot 'sumi-app'
$OutDir = if ($Path) { $Path } else { Join-Path $RepoRoot 'out' }

# Electron 包（npm install）与 electron-builder（pack）的二进制下载默认走 npmmirror（国内网络）；
# 用户已设置同名环境变量时尊重用户配置
$env:ELECTRON_MIRROR = if ($env:ELECTRON_MIRROR) { $env:ELECTRON_MIRROR } else { 'https://npmmirror.com/mirrors/electron/' }
$env:ELECTRON_BUILDER_BINARIES_MIRROR = if ($env:ELECTRON_BUILDER_BINARIES_MIRROR) { $env:ELECTRON_BUILDER_BINARIES_MIRROR } else { 'https://npmmirror.com/mirrors/electron-builder-binaries/' }

Push-Location $AppDir
try {
    if (-not (Test-Path (Join-Path $AppDir 'node_modules'))) { npm install }
    if ($Unpacked) {
        npm run pack:dir
        Write-Host "未打包产物: $AppDir\dist\"
        return
    }
    npm run pack

    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
    $zip = Get-ChildItem (Join-Path $AppDir 'dist') -Filter 'sumi-windows-*.zip' | Select-Object -First 1
    if (-not $zip) { throw '打包产物不存在: dist/sumi-windows-*.zip（electron-builder 未产出）' }
    Copy-Item $zip.FullName (Join-Path $OutDir $zip.Name) -Force
    Write-Host "应用分发包: $(Join-Path $OutDir $zip.Name)"
} finally {
    Pop-Location
}
