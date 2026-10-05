$ErrorActionPreference = 'Stop'
$truhabitRoot = Split-Path -Parent $PSScriptRoot
$cargoExe = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
if (-not (Test-Path -LiteralPath $cargoExe)) {
    $cargoExe = (Get-Command cargo -ErrorAction Stop).Source
}
$sbfCommand = Get-Command cargo-build-sbf -ErrorAction SilentlyContinue
if ($sbfCommand) {
    $sbfExe = $sbfCommand.Source
} else {
    $sbfExe = Join-Path $truhabitRoot '.tools/solana-build/bin/cargo-build-sbf.exe'
}
if (-not (Test-Path -LiteralPath $sbfExe)) {
    throw 'Install the reviewed cargo-build-sbf toolchain first; see docs/DEVELOPMENT.md. This verifier does not run an installer.'
}

$previousPath = $env:PATH
$previousLocation = Get-Location
$previousToolchain = $env:RUSTUP_TOOLCHAIN
try {
    Set-Location -LiteralPath $truhabitRoot
    $env:PATH = "$(Split-Path -Parent $cargoExe);$previousPath"
    $env:RUSTUP_TOOLCHAIN = $null
    foreach ($manifest in @('contracts/prototype/Cargo.toml', 'contracts/prototype-tests/Cargo.toml')) {
        Write-Host "Checking contract sources: $manifest"
        & $cargoExe fmt --manifest-path $manifest --all -- --check
        if ($LASTEXITCODE -ne 0) { throw "Formatting failed: $manifest" }
        & $cargoExe clippy --manifest-path $manifest --all-targets --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw "Clippy failed: $manifest" }
    }

    # Execute fresh SBF from the active program, not a previously built artifact.
    Write-Host 'Building the active prototype SBF program.'
    & $sbfExe --manifest-path contracts/prototype/programs/truhabit-prototype/Cargo.toml --tools-version v1.57 -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Prototype SBF compilation failed.' }
    & $cargoExe test --manifest-path contracts/prototype-tests/Cargo.toml --locked --test prototype -- --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'Actual prototype SBF transaction tests failed.' }

    Write-Host 'Generating and checking the active program interface.'
    & $cargoExe build --manifest-path contracts/prototype-tests/Cargo.toml --locked --example build_prototype_idl
    if ($LASTEXITCODE -ne 0) { throw 'Prototype IDL builder compilation failed.' }
    & './contracts/prototype-tests/target/debug/examples/build_prototype_idl.exe'
    if ($LASTEXITCODE -ne 0) { throw 'Prototype IDL generation or interface validation failed.' }
    $generatedIdlPath = 'contracts/prototype/target/idl/truhabit_prototype.json'
    $sourceIdlPath = 'contracts/prototype/idl/truhabit_prototype.json'
    $idl = Get-Content -LiteralPath $generatedIdlPath -Raw | ConvertFrom-Json
    $sourceIdl = Get-Content -LiteralPath $sourceIdlPath -Raw | ConvertFrom-Json
    if (($idl | ConvertTo-Json -Depth 100 -Compress) -cne ($sourceIdl | ConvertTo-Json -Depth 100 -Compress)) {
        throw 'Generated prototype IDL differs from the checked-in interface. Review the interface change before release.'
    }

    $suffix = [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $output = Join-Path $truhabitRoot ("output/contracts/" + (Get-Date -Format 'yyyyMMdd-HHmmss') + "-$suffix")
    New-Item -ItemType Directory -Path $output | Out-Null
    # Only public artifacts are copied; deployment keypairs stay in ignored build output.
    Copy-Item -LiteralPath 'contracts/prototype/target/deploy/truhabit_prototype.so' -Destination $output
    Copy-Item -LiteralPath $generatedIdlPath -Destination $output
    $sources = @(
        'rust-toolchain.toml', 'contracts/prototype/Cargo.toml', 'contracts/prototype/Cargo.lock',
        'contracts/prototype/programs/truhabit-prototype/Cargo.toml',
        'contracts/prototype/programs/truhabit-prototype/src/lib.rs', $sourceIdlPath,
        'contracts/prototype-tests/Cargo.toml', 'contracts/prototype-tests/Cargo.lock',
        'contracts/prototype-tests/tests/prototype.rs',
        'contracts/prototype-tests/examples/build_prototype_idl.rs', 'scripts/verify-contract.ps1'
    )
    $sourceHashes = @($sources | ForEach-Object {
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $artifactHashes = @(@('truhabit_prototype.so', 'truhabit_prototype.json') | ForEach-Object {
        $path = Join-Path $output $_
        @{ path = $_; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant(); bytes = (Get-Item -LiteralPath $path).Length }
    })
    $report = [ordered]@{
        verified_at_utc = (Get-Date).ToUniversalTime().ToString('o')
        environment = 'native Windows x64; local LiteSVM; no deployment or chain transactions'
        rust_host = (& $cargoExe --version)
        cargo_build_sbf = ((& $sbfExe --version) -join "`n")
        platform_tools = 'v1.57'; anchor = '1.2.0'; litesvm = '0.16.0'
        program_id = $idl.address; instruction_count = $idl.instructions.Count
        checks = @('rustfmt', 'clippy -D warnings', 'active prototype SBF compilation', 'actual prototype SBF transaction suite', 'compiled IDL generation', 'checked-in IDL equality')
        production_approved = $false
        source_hashes = $sourceHashes; artifacts = $artifactHashes
    }
    if ($LASTEXITCODE -ne 0) { throw 'SBF tool version could not be read.' }
    $report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $output 'verification.json') -Encoding UTF8
    Write-Host "Active contract checks passed. Public artifacts only: $output"
} finally {
    $env:PATH = $previousPath
    $env:RUSTUP_TOOLCHAIN = $previousToolchain
    Set-Location -LiteralPath $previousLocation
}
