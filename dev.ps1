<#
Local development commands for this IronCalc fork on Windows.

  .\dev.ps1 setup       Install web dependencies and build everything
  .\dev.ps1 start       Run the full app at http://localhost:5180
  .\dev.ps1 storybook   Work on the spreadsheet widget alone at http://localhost:6006
  .\dev.ps1 build       Rebuild the wasm engine and the widget
  .\dev.ps1 test        Run the Rust, wasm and web tests
  .\dev.ps1 lint        Check formatting and lint rules
  .\dev.ps1 format      Fix formatting
  .\dev.ps1 sync        Merge the latest upstream IronCalc into main and rebuild
#>
param(
  [Parameter(Position = 0)]
  [ValidateSet('help', 'setup', 'start', 'storybook', 'build', 'test', 'lint', 'format', 'sync')]
  [string]$Command = 'help'
)

$Root = $PSScriptRoot
$Webapp = Join-Path $Root 'webapp'
$Widget = Join-Path $Webapp 'IronCalc'
$Frontend = Join-Path $Webapp 'app.ironcalc.com\frontend'
$Embed = Join-Path $Webapp 'embed.ironcalc.com'
$Server = Join-Path $Webapp 'app.ironcalc.com\server'

# The upstream Makefiles need POSIX tools (sh, sort, cp, rm), which Git for
# Windows ships but does not put on PATH.
$gitExe = (Get-Command git -ErrorAction Stop).Source
$posixTools = Join-Path (Split-Path (Split-Path $gitExe)) 'usr\bin'
$env:Path = "$posixTools;$env:Path"

# Takes the command as one string: PowerShell would otherwise drop '--' and
# treat flags such as -p as its own parameters.
function Run([string]$Dir, [string]$CommandLine) {
  Write-Host "`n> [$(Split-Path $Dir -Leaf)] $CommandLine" -ForegroundColor Cyan
  $parts = $CommandLine -split ' '
  $exe = $parts[0]
  $arguments = @($parts | Select-Object -Skip 1)
  Push-Location $Dir
  try {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "'$CommandLine' failed with exit code $LASTEXITCODE" }
  } finally {
    Pop-Location
  }
}

function Install-WebDeps { Run $Webapp 'pnpm install' }

# make skips this when the Rust sources have not changed.
function Build-Wasm { Run $Root 'make -C bindings/wasm wasm' }

function Build-Widget {
  Run $Widget 'pnpm exec vite build'
  Run $Widget 'pnpm exec tsc -p tsconfig.json'
}

function Build-All {
  Build-Wasm
  Install-WebDeps
  Build-Widget
}

function Remove-RustTestFiles {
  foreach ($name in 'hello-calc.xlsx', 'hello-styles.xlsx', 'widths-and-heights.xlsx') {
    Remove-Item (Join-Path $Root "xlsx\$name") -ErrorAction SilentlyContinue
  }
}

function Start-Background([string]$Name, [string]$Dir, [string]$Exe, [string[]]$Arguments) {
  $logs = Join-Path $Root 'target\dev'
  New-Item -ItemType Directory -Force $logs | Out-Null
  Start-Process -FilePath $Exe -ArgumentList $Arguments -WorkingDirectory $Dir -NoNewWindow -PassThru `
    -RedirectStandardOutput (Join-Path $logs "$Name.log") -RedirectStandardError (Join-Path $logs "$Name.err.log")
}

function Start-App {
  Build-All

  # ironcalc.sqlite is tracked in git, so the dev server uses a private copy.
  $devDir = Join-Path $Root 'target\dev'
  New-Item -ItemType Directory -Force $devDir | Out-Null
  $devDb = Join-Path $devDir 'ironcalc.sqlite'
  if (-not (Test-Path $devDb)) { Copy-Item (Join-Path $Server 'ironcalc.sqlite') $devDb }
  $env:ROCKET_DATABASES = '{ironcalc={url="../../../target/dev/ironcalc.sqlite"}}'

  Write-Host "`nCompiling the backend (slow the first time)..." -ForegroundColor Cyan
  Run $Server 'cargo build'

  $children = @(
    (Start-Background 'server' $Server 'cargo' @('run')),
    (Start-Background 'widget' $Widget 'pnpm.cmd' @('exec', 'vite', 'build', '--watch', '--emptyOutDir=false'))
  )
  Write-Host "`nApp: http://localhost:5180   Backend and widget logs: target\dev   Stop: Ctrl+C" -ForegroundColor Green
  Write-Host "After changing Rust engine code, run '.\dev.ps1 build' in another terminal." -ForegroundColor Green
  try {
    Run $Frontend 'pnpm exec vite --config vite.dev.config.ts --port 5180 --strictPort'
  } finally {
    foreach ($child in $children) { cmd /c "taskkill /PID $($child.Id) /T /F >nul 2>&1" }
  }
}

function Test-All {
  # The Windows GNU toolchain's sin/cos lose precision at inputs around 1e10,
  # which is enough to fail these four high-precision tests on this machine only.
  Run $Root 'cargo test -p ironcalc_base -p ironcalc -- --skip bessel_j0_known_values --skip bessel_j1_known_values --skip bessel_y0_known_values --skip bessel_yn_known_values'
  Write-Warning 'Skipped 4 Bessel precision tests that cannot pass with the Windows GNU toolchain.'
  Remove-RustTestFiles
  Run $Root 'make -C bindings/wasm tests'
  Build-All
  Run $Widget 'pnpm run typecheck'
  Run $Widget 'pnpm exec vitest run --exclude tests/worksheetCanvas/renderSheet.test.ts'
  if (Get-Command docker -ErrorAction SilentlyContinue) {
    Run $Widget 'pnpm run test:visual'
  } else {
    Write-Warning 'Skipped the visual tests: they need Docker, which is not installed.'
  }
  Run $Frontend 'pnpm run typecheck'
  Run $Frontend 'pnpm run test'
  Run $Embed 'pnpm run typecheck'
}

function Lint-All {
  Run $Root 'cargo fmt -- --check'
  # The Node.js bindings need libnode.dll to build with the Windows GNU toolchain.
  Run $Root 'cargo clippy --workspace --exclude ironcalc_nodejs --all-targets --all-features -- -W clippy::unwrap_used -W clippy::expect_used -W clippy::panic -D warnings'
  Install-WebDeps
  Run $Widget 'pnpm exec biome check ./src ./tests .storybook/ vite.config.ts package.json tsconfig.json tsconfig.node.json tsconfig.test.json'
  Run $Widget 'pnpm run lint:i18n'
  Run $Frontend 'pnpm run check'
  Run $Embed 'pnpm exec biome check ./src index.html example.html package.json'
}

function Format-All {
  Run $Root 'cargo fmt'
  Run $Widget 'pnpm exec biome check --write ./src ./tests .storybook/'
  Run $Frontend 'pnpm exec biome check --write ./src'
  Run $Embed 'pnpm exec biome check --write ./src'
}

function Sync-Upstream {
  $branch = (git -C $Root rev-parse --abbrev-ref HEAD).Trim()
  if ($branch -ne 'main') { throw "Switch to main first (currently on '$branch')." }
  if (git -C $Root status --porcelain --untracked-files=no) { throw 'Commit or stash your changes first.' }
  Run $Root 'git fetch upstream'
  Run $Root 'git merge --no-edit upstream/main'
  Build-All
  Write-Host "`nMerged and rebuilt. Publish it to your fork with: git push origin main" -ForegroundColor Green
}

switch ($Command) {
  'setup' { Build-All }
  'start' { Start-App }
  'storybook' { Build-All; Run $Widget 'pnpm run storybook' }
  'build' { Build-Wasm; Build-Widget }
  'test' { Test-All }
  'lint' { Lint-All }
  'format' { Format-All }
  'sync' { Sync-Upstream }
  default { Get-Content $PSCommandPath -TotalCount 11 | Select-Object -Skip 1 }
}
