# Installs FastSSH from its Windows installer the way a user would, starts it,
# and checks that the app stays up and its built-in server works.
#
# The release workflow runs this before publishing, because nobody tests the
# Windows build by hand. It expects a machine with no FastSSH profile yet.
param(
    [Parameter(Mandatory)] [string] $Installer
)

$ErrorActionPreference = 'Stop'

$installDir = Join-Path ([System.IO.Path]::GetTempPath()) 'fastssh-smoke-test'

# /S installs silently; /D picks the folder and has to come last, unquoted.
$setup = Start-Process -FilePath $Installer -ArgumentList '/S', "/D=$installDir" -Wait -PassThru
if ($setup.ExitCode -ne 0) { throw "the installer failed with code $($setup.ExitCode)" }

$exe = Join-Path $installDir 'fastssh-desktop.exe'
if (-not (Test-Path $exe)) {
    Get-ChildItem $installDir -Recurse -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName | Write-Host
    throw "the installer did not put the app at $exe"
}

$app = Start-Process -FilePath $exe -PassThru
try {
    # The app serves its interface on a free localhost port. Wait for it.
    $port = $null
    foreach ($attempt in 1..30) {
        Start-Sleep -Seconds 1
        if ($app.HasExited) { throw 'the app exited right after starting' }
        $listener = Get-NetTCPConnection -OwningProcess $app.Id -State Listen -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($listener) { $port = $listener.LocalPort; break }
    }
    if (-not $port) { throw 'the app never started listening' }
    $base = "http://127.0.0.1:$port"
    Write-Host "FastSSH is serving its interface at $base"

    $page = Invoke-WebRequest "$base/" -UseBasicParsing
    if ($page.Content -notmatch '<title>FastSSH</title>') { throw 'the interface was not served' }

    $session = Invoke-RestMethod "$base/api/session"
    if ($session.state -ne 'setup') { throw "expected a fresh profile, found state '$($session.state)'" }

    # Create the profile the way the first-run screen does, then read it back.
    Invoke-RestMethod "$base/api/signup" -Method Post -ContentType 'application/json' `
        -Body '{"password":"smoke test password"}' -SessionVariable web | Out-Null
    $session = Invoke-RestMethod "$base/api/session" -WebSession $web
    Write-Host "Session after creating the profile: $($session | ConvertTo-Json -Compress)"
    if ($session.state -ne 'ready' -or -not $session.desktop) { throw 'the profile was not created' }
    if ($session.local_shell) { throw 'the local shell is not built for Windows yet and should be off' }

    # A window that fails to open ends the process, so give it time to.
    Start-Sleep -Seconds 10
    if ($app.HasExited) { throw 'the app exited while opening its window' }
    Write-Host 'FastSSH installed, started and stayed up.'
}
finally {
    if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
}
