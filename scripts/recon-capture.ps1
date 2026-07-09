# Captures whatever Claude Code pipes to a statusline / hook command.
# Used by the recon project (D:\code\explore\claude-recon) to validate field names.
#
# Invoked from that project's .claude/settings.json as either:
#   powershell -NoProfile -ExecutionPolicy Bypass -File <this>.ps1 statusline
#   powershell -NoProfile -ExecutionPolicy Bypass -File <this>.ps1 hook
param([string]$Kind = "statusline")
$ErrorActionPreference = "SilentlyContinue"
$out = "D:\code\explore\claude-task-state\recon-out"
New-Item -ItemType Directory -Force -Path $out | Out-Null
$raw = [Console]::In.ReadToEnd()
$stamp = (Get-Date).ToString("yyyyMMdd-HHmmssfff")
if ($Kind -eq "hook") {
    try { $obj = $raw | ConvertFrom-Json; $tag = ($obj.hook_event_name) } catch { $tag = "unknown" }
    if (-not $tag) { $tag = "unknown" }
    $file = Join-Path $out ("hook-" + $tag + "-" + $stamp + ".json")
} else {
    $file = Join-Path $out ("statusline-" + $stamp + ".json")
}
[IO.File]::WriteAllText($file, $raw, [Text.Encoding]::UTF8)
# statusline commands must print a line (it becomes the TUI status line); hooks must stay silent.
if ($Kind -eq "statusline") { Write-Output "recon-capture" }
