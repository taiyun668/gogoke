# R2-06b Owner 私钥本机签署输入

签署前在本仓库根目录核对下表。两份 `.input.md` 均为 ASCII、仅 LF 换行且末尾恰有一个 LF；离线副本的文件名必须是 `SHA256SUMS.windows`。本步骤只产生正式清单签名，不发布、不接受 R2-06，也不触碰正式安装。

| 种类 | 源提交和云端冻结 artifact | 待签输入 | 字节数 | SHA-256 |
| --- | --- | --- | ---: | --- |
| `full` 0.1.3 | `1ffda964807d257c0bc9fddfec03023642395b9c`，run `36359270160`，artifact `10945084785` | `full-0.1.3/SHA256SUMS.windows.input.md` | 456 | `33d8edb752d1ea09e9e087e5cf21e505093e5d85743419e17f90ce275802c85e` |
| `resources` 0.1.4 | `309d7f4dad924173e95e52971029e4bf92baa9ac`，run `36363049008`，artifact `10947590712` | `resources-0.1.4/SHA256SUMS.windows.input.md` | 238 | `8ff4d4803a949306bc46d85890db740d3c71966e52de3f6dc9de46fb6f180641` |

`full` 的四个资产 SHA-256，按清单顺序：

| 资产 | SHA-256 |
| --- | --- |
| `resource-index.json` | `84f53efa4512981990e7a5786a9fd71bfaf73994d8ddfea0bfe67cde39a16fed` |
| `gogoke-resources.windows.zip` | `fb5359e2f656cb638f50367cdccc9be4cefe801eeae04f5feefe3cd65433dc22` |
| `gogoke-0.1.3-windows-x64-unsigned-setup.exe` | `bdf9b6e000a8874dd7672fda8d5d83543b82f93032d654e6ab393427208c1e2c` |
| `gogoke-0.1.3-windows-x64-unsigned-portable.zip` | `7075fdf4eadded408b3fd33ce93ac1e8aefd0b708160dfcd000fe75ac7b7114b` |

`resources` 的清单恰有两项，没有安装程序：

| 资产 | SHA-256 |
| --- | --- |
| `resource-index.json` | `c7e79b9c7d4d942401ce5ab11e79b54f0e2c5088f112a975082d2640d8b928bf` |
| `gogoke-resources.windows.zip` | `63a33ef53cfc8587a171ac8fff7e6f596e70b74577b6ee6db56bba4cdcd7cb99` |

Owner 于 2026-09-25 授权 Controller 在本机从仓库根目录运行下列 PowerShell。脚本默认从 `%USERPROFILE%\.gogoke\release-key.txt` 读取 Owner 私钥；私钥不进入仓库、CI、日志或任何产物。不要使用 `-NewKey`。公开发布仍须 Owner 决定。

```powershell
$repo = (Resolve-Path .).Path
$inputRoot = Join-Path $repo 'artifacts\s1-r4\release-inputs\R2-06b'
$fullInput = Join-Path $inputRoot 'full-0.1.3\SHA256SUMS.windows.input.md'
$resourcesInput = Join-Path $inputRoot 'resources-0.1.4\SHA256SUMS.windows.input.md'
if ((Get-FileHash -LiteralPath $fullInput -Algorithm SHA256).Hash.ToLowerInvariant() -cne '33d8edb752d1ea09e9e087e5cf21e505093e5d85743419e17f90ce275802c85e') { throw 'full input changed' }
if ((Get-FileHash -LiteralPath $resourcesInput -Algorithm SHA256).Hash.ToLowerInvariant() -cne '8ff4d4803a949306bc46d85890db740d3c71966e52de3f6dc9de46fb6f180641') { throw 'resources input changed' }

$offlineRoot = Join-Path $env:USERPROFILE '.gogoke\offline-r206b'
$fullDir = Join-Path $offlineRoot 'full-0.1.3'
$resourcesDir = Join-Path $offlineRoot 'resources-0.1.4'
if ((Test-Path -LiteralPath $fullDir) -or (Test-Path -LiteralPath $resourcesDir)) { throw 'offline output already exists' }
New-Item -ItemType Directory -Path $fullDir,$resourcesDir -Force | Out-Null
$fullManifest = Join-Path $fullDir 'SHA256SUMS.windows'
$resourcesManifest = Join-Path $resourcesDir 'SHA256SUMS.windows'
Copy-Item -LiteralPath $fullInput -Destination $fullManifest
Copy-Item -LiteralPath $resourcesInput -Destination $resourcesManifest
if ((Get-FileHash -LiteralPath $fullManifest -Algorithm SHA256).Hash.ToLowerInvariant() -cne '33d8edb752d1ea09e9e087e5cf21e505093e5d85743419e17f90ce275802c85e') { throw 'full copy changed' }
if ((Get-FileHash -LiteralPath $resourcesManifest -Algorithm SHA256).Hash.ToLowerInvariant() -cne '8ff4d4803a949306bc46d85890db740d3c71966e52de3f6dc9de46fb6f180641') { throw 'resources copy changed' }

& (Join-Path $repo 'tools\sign-gogoke-release-manifest.ps1') -Manifest $fullManifest -Version 0.1.3 -ReleaseType full
& (Join-Path $repo 'tools\sign-gogoke-release-manifest.ps1') -Manifest $resourcesManifest -Version 0.1.4 -ReleaseType resources

if ((Get-FileHash -LiteralPath $fullManifest -Algorithm SHA256).Hash.ToLowerInvariant() -cne '33d8edb752d1ea09e9e087e5cf21e505093e5d85743419e17f90ce275802c85e') { throw 'signed full manifest bytes changed' }
if ((Get-FileHash -LiteralPath $resourcesManifest -Algorithm SHA256).Hash.ToLowerInvariant() -cne '8ff4d4803a949306bc46d85890db740d3c71966e52de3f6dc9de46fb6f180641') { throw 'signed resources manifest bytes changed' }
if (-not (Test-Path -LiteralPath "$fullManifest.sig") -or -not (Test-Path -LiteralPath "$resourcesManifest.sig")) { throw 'signature missing' }
```

签署后保留仓库外的两份清单及两份签名；后续验签、同字节正式测试和任何发布决定分别处理。签名本身不构成 R2-06 验收或公开发布。
