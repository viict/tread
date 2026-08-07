# Install tread.
#
#   irm https://raw.githubusercontent.com/viict/tread/master/install.ps1 | iex
#
# Environment:
#   $env:INSTALL_PATH   where to put tread.exe (default: %LOCALAPPDATA%\Programs\tread)
#   $env:VERSION        which release to install (default: the latest)
#
# The counterpart of install.sh, and it behaves the same way: pick the build for
# this machine, verify it against the release's SHA256SUMS, refuse to install
# anything that does not match, and leave nothing behind either way.
#
# Everything is inside Install-Tread, called on the last line. `iex` parses the
# whole string before running any of it, so a download cut off mid-transfer is a
# parse error that does nothing at all rather than half an install. Nothing here
# reads $MyInvocation or $PSScriptRoot: there is no script on disk.
#
# Targets Windows PowerShell 5.1 (what ships with Windows) and PowerShell 7.
# Failure is a `throw`, never `exit`: `exit` inside `iex` would close the
# console the user typed the one-liner into. An uncaught throw still leaves a
# non-zero exit code when the one-liner is run non-interactively.

function Install-Tread {
    [CmdletBinding()]
    param()

    $Repo = 'viict/tread'
    $UserAgent = 'tread-install'
    $ErrorActionPreference = 'Stop'
    # Scoped to this function, so it changes nothing about the session it was
    # pasted into. Nothing below wants strict mode, and a user who runs with it
    # on should not get a different installer.
    Set-StrictMode -Off
    # Invoke-WebRequest spends most of a download redrawing this on 5.1.
    $ProgressPreference = 'SilentlyContinue'

    # Expand-Archive arrived in PowerShell 5.0, Get-FileHash in 4.0. Windows 10
    # ships 5.1, but Server 2012 R2 and stripped images do not.
    if ($PSVersionTable.PSVersion.Major -lt 5) {
        throw "install: this needs PowerShell 5.1 or newer (found $($PSVersionTable.PSVersion))"
    }
    # PowerShell 7 also runs on macOS and Linux, where every assumption below is
    # wrong. Windows PowerShell has no $IsWindows because it never needed one.
    if ($PSVersionTable.PSEdition -eq 'Core' -and -not $IsWindows) {
        throw "install: this is the Windows installer; on macOS or Linux use install.sh"
    }

    # Windows PowerShell defaults to whatever SecurityProtocol the .NET
    # Framework was configured with, which on an unpatched box is still SSL3 +
    # TLS 1.0 -- and github.com has required TLS 1.2 for years. PowerShell 7
    # leaves this to the OS and must not be touched. Or'ed in, so nothing the
    # user or the machine policy already enabled is taken away.
    if ($PSVersionTable.PSEdition -ne 'Core') {
        try {
            [Net.ServicePointManager]::SecurityProtocol =
                [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        } catch {
            Write-Warning "install: could not enable TLS 1.2; the download may fail"
        }
    }

    # $env:PROCESSOR_ARCHITECTURE describes the *process*, so a 32-bit
    # PowerShell on a 64-bit machine says x86 -- and an x64 PowerShell emulated
    # on an ARM64 machine says AMD64. Under WOW64 the machine's real
    # architecture is in PROCESSOR_ARCHITEW6432, which is absent otherwise, so
    # preferring it is right in every combination and installs the native build
    # rather than the emulated one.
    function Get-Triple {
        $arch = $env:PROCESSOR_ARCHITEW6432
        if (-not $arch) { $arch = $env:PROCESSOR_ARCHITECTURE }
        if (-not $arch -or $arch -eq 'x86') {
            # Belt and braces for a shell that inherited a scrubbed environment.
            # Absent before .NET Framework 4.7.1; a probe, so nothing to report.
            try { $arch = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() } catch { }
        }
        if (-not $arch) { throw "install: cannot tell what architecture this machine is" }
        switch -Regex ($arch) {
            '^(AMD64|x64|EM64T)$' { return 'x86_64-pc-windows-msvc' }
            '^ARM64$'             { return 'aarch64-pc-windows-msvc' }
            '^(x86|ARM)$'         { throw "install: 32-bit Windows ($arch) is not built; tread ships x64 and ARM64 only" }
            default               { throw "install: unsupported architecture: $arch" }
        }
    }

    # The newest release tag. Invoke-RestMethod parses the JSON, so there is no
    # hand-rolled parsing here the way install.sh needs one to avoid jq.
    function Get-LatestVersion {
        try {
            $r = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" `
                -UserAgent $UserAgent -Headers @{ Accept = 'application/vnd.github+json' } `
                -UseBasicParsing
        } catch {
            throw "install: could not reach the GitHub API to find the latest release"
        }
        if (-not $r.tag_name) { throw "install: could not find a release; set `$env:VERSION = 'vX.Y.Z' to choose one" }
        return [string]$r.tag_name
    }

    # $true / $false rather than an exception, so a missing asset and a missing
    # SHA256SUMS can be reported differently. A failed transfer can still have
    # written a prefix of the file; it is deleted so nothing downstream sees it.
    function Get-File([string]$Url, [string]$Path) {
        try {
            Invoke-WebRequest -Uri $Url -OutFile $Path -UserAgent $UserAgent -UseBasicParsing
        } catch {
            Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
            return $false
        }
        return (Test-Path -LiteralPath $Path)
    }

    # Verifying is the point of publishing checksums, so a SHA256SUMS that is
    # missing, or does not list this file, is a refusal and not a silent skip.
    # Lines are `<64 hex>  <name>`; GNU sha256sum writes two spaces, and a `*`
    # marks binary mode in some implementations.
    function Assert-Checksum([string]$Archive, [string]$Sums, [string]$Name) {
        $want = $null
        foreach ($line in (Get-Content -LiteralPath $Sums)) {
            if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(\S.*?)\s*$') {
                if ($Matches[2] -eq $Name) { $want = $Matches[1].ToLowerInvariant(); break }
            }
        }
        if (-not $want) { throw "install: $Name is not listed in SHA256SUMS" }
        $got = (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($want -ne $got) {
            throw "install: checksum mismatch for $Name (expected $want, got $got) - refusing to install"
        }
    }

    # Windows will not let a running image be overwritten, but it will let one
    # be renamed. So: stage the new exe under a name nothing can hold open, move
    # any existing tread.exe aside, then move the new one into place -- the same
    # "write beside it and rename over" the shell installer does, and a running
    # tread keeps working off the renamed file until it exits. If even the
    # rename is refused, say so plainly instead of leaving a half-written exe.
    function Install-Exe([string]$Src, [string]$Dest) {
        $final = Join-Path $Dest 'tread.exe'
        $stage = Join-Path $Dest 'tread.exe.new'
        # Sweep anything a previous run could not delete because it was in use.
        Get-ChildItem -LiteralPath $Dest -Filter 'tread.exe.old-*' -ErrorAction SilentlyContinue |
            ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue }

        Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue
        try { Copy-Item -LiteralPath $Src -Destination $stage -Force }
        catch { throw "install: could not write to $Dest - $($_.Exception.Message)" }

        $aside = $null
        if (Test-Path -LiteralPath $final) {
            $aside = Join-Path $Dest ('tread.exe.old-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))
            try { Move-Item -LiteralPath $final -Destination $aside -Force }
            catch {
                Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue
                throw "install: $final is in use and cannot be replaced - close every running tread and try again"
            }
        }
        try { Move-Item -LiteralPath $stage -Destination $final -Force }
        catch {
            # Put the old one back rather than leaving the machine with none.
            if ($aside) { Move-Item -LiteralPath $aside -Destination $final -Force -ErrorAction SilentlyContinue }
            Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue
            throw "install: could not install $final - $($_.Exception.Message)"
        }
        # Still open if a tread is running; the next install sweeps it up.
        if ($aside) { Remove-Item -LiteralPath $aside -Force -ErrorAction SilentlyContinue }
        return $final
    }

    # The persistent user PATH, unexpanded, for the "is it already there?" test
    # only -- nothing here writes it. Reading it through [Environment] returns it
    # already expanded, so a %USERPROFILE% in the registry would come back as
    # today's directory; the registry provider can read the raw string, which is
    # what the printed advice should be compared against.
    #
    # $null when the value cannot be read at all, which is reported as "not on
    # PATH" rather than as an error: the worst outcome is advice the user did not
    # need.
    function Get-UserPath {
        try {
            $key = Get-Item -LiteralPath 'HKCU:\Environment'
            $opt = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
            return [string]$key.GetValue('Path', '', $opt)
        } catch {
            try { return [string][Environment]::GetEnvironmentVariable('Path', 'User') } catch { return $null }
        }
    }

    # SPEC.md §"Installing on Windows": *report* how to add the directory to PATH
    # when it is not already there. Same contract as install.sh, which prints an
    # `export PATH=...` line and changes nothing.
    #
    # This deliberately does not write HKCU:\Environment. A one-liner piped from
    # the internet editing the user's persistent environment is more than was
    # asked for, and it is more than the Unix half does -- and it could not even
    # be honest about it: a registry write is invisible to every process already
    # running unless WM_SETTINGCHANGE is broadcast, so "open a new terminal for
    # that to take effect" was false for any terminal launched from the Explorer
    # session that was already up. Printing the command the user runs themselves
    # has no such gap: they run it, in their shell, and can see what it did.
    function Show-PathAdvice([string]$Dest) {
        $target = $Dest.TrimEnd('\')
        # This process's PATH covers the common case; the persistent value covers
        # a shell that was started before an earlier install.
        $seen = @($env:PATH -split ';')
        $stored = Get-UserPath
        if ($stored) { $seen += [Environment]::ExpandEnvironmentVariables($stored) -split ';' }
        foreach ($p in $seen) {
            if ($p -and $p.Trim('"').TrimEnd('\') -ieq $target) { return }
        }
        Write-Host ''
        Write-Host "$target is not on your PATH. Add it with:"
        Write-Host ''
        # Not setx: it truncates at 1024 characters, and it expands the value it
        # writes -- which would bake today's %USERPROFILE% into the registry.
        Write-Host "    [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path','User') + ';$target', 'User')"
        Write-Host ''
        Write-Host 'then open a new terminal.'
    }

    $dest = $env:INSTALL_PATH
    if (-not $dest) {
        $local = $env:LOCALAPPDATA
        if (-not $local -and $env:USERPROFILE) { $local = Join-Path $env:USERPROFILE 'AppData\Local' }
        if (-not $local) { throw "install: %LOCALAPPDATA% is not set; set `$env:INSTALL_PATH to choose a directory" }
        $dest = Join-Path $local 'Programs\tread'
    }

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('tread-install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp -Force | Out-Null
    try {
        $triple = Get-Triple
        $version = $env:VERSION
        if (-not $version) { $version = Get-LatestVersion }
        $name = "tread-$version-$triple"
        $base = "https://github.com/$Repo/releases/download/$version"
        $zip = Join-Path $tmp "$name.zip"
        $sums = Join-Path $tmp 'SHA256SUMS'

        Write-Host "tread $version for $triple"

        if (-not (Get-File "$base/$name.zip" $zip)) {
            throw "install: no build for $triple in $version - see https://github.com/$Repo/releases"
        }
        if (-not (Get-File "$base/SHA256SUMS" $sums)) {
            throw "install: could not download SHA256SUMS - refusing to install unverified"
        }
        Assert-Checksum -Archive $zip -Sums $sums -Name "$name.zip"
        Write-Host 'checksum ok'

        Expand-Archive -LiteralPath $zip -DestinationPath $tmp -Force
        $exe = Join-Path (Join-Path $tmp $name) 'tread.exe'
        if (-not (Test-Path -LiteralPath $exe)) { throw "install: the archive does not contain tread.exe" }

        if (-not (Test-Path -LiteralPath $dest)) {
            try { New-Item -ItemType Directory -Path $dest -Force | Out-Null }
            catch { throw "install: could not create $dest - $($_.Exception.Message)" }
        }
        $final = Install-Exe $exe $dest
        # The download carries no mark of the web, but an archive that did would
        # pass it to what came out of it; clearing it is free either way.
        try { Unblock-File -LiteralPath $final } catch { <# not NTFS; nothing to clear #> }

        Write-Host "installed $final"
        Show-PathAdvice $dest
        & $final --version
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Called, then taken back out of the session it was defined in: `iex` runs in
# the caller's scope, and a one-liner should not leave a function behind.
try { Install-Tread } finally { Remove-Item -LiteralPath 'function:Install-Tread' -ErrorAction SilentlyContinue }
