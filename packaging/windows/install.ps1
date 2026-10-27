# Adds Leaf to this account's "Open with" lists, with its icon, for books
# and images, and "Open in Leaf" to folders' right-click menu. Comic
# archives (.cbz, .cbr, .cb7, .cbt) open in Leaf by double-click, unless
# another app already opens them. Only your own account's settings change
# (HKEY_CURRENT_USER); nothing needs an administrator.
#
#   install.cmd            add Leaf (again, after moving this folder)
#   install.cmd --remove   take it out again
param([string]$Action = "")
$ErrorActionPreference = "Stop"

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here "Leaf.exe"
$icon = Join-Path $here "leaf.ico"
$classes = "HKCU:\Software\Classes"
$comics = ".cbz", ".cbr", ".cb7", ".cbt"
$others = ".zip", ".epub", ".pdf", ".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"
$ours = "Leaf.Comic", "Leaf.Book", "Applications\Leaf.exe", "Directory\shell\Leaf"

# Explorer reads file types again only when told they changed.
function Refresh {
    Add-Type -Namespace Leaf -Name Shell -MemberDefinition '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);'
    [Leaf.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
}

# A key's default value, or "" when there's none.
function Default($key) {
    if (Test-Path -LiteralPath $key) { [string](Get-Item -LiteralPath $key).GetValue("") } else { "" }
}

function Set-Default($key, $value) {
    New-Item -Path $key -Force | Out-Null
    Set-Item -LiteralPath $key -Value $value
}

if ($Action -eq "--remove" -or $Action -eq "/remove") {
    foreach ($k in $ours) { Remove-Item -LiteralPath "$classes\$k" -Recurse -Force -ErrorAction SilentlyContinue }
    foreach ($ext in $comics + $others) {
        Remove-ItemProperty -LiteralPath "$classes\$ext\OpenWithProgids" -Name "Leaf.Comic", "Leaf.Book" -ErrorAction SilentlyContinue
        if ((Default "$classes\$ext") -eq "Leaf.Comic") { Remove-ItemProperty -LiteralPath "$classes\$ext" -Name "(default)" }
    }
    Refresh
    Write-Output "Leaf is out of the Open with lists. (This folder is untouched; delete it to remove Leaf.)"
    exit 0
}

if (-not (Test-Path -LiteralPath $exe)) { Write-Error "Leaf.exe isn't next to install.cmd"; exit 1 }
$open = '"' + $exe + '" "%1"'

# The two kinds of file Leaf opens, each with Leaf's icon and command.
foreach ($kind in @(("Leaf.Comic", "Comic book"), ("Leaf.Book", "Book or image"))) {
    $k = "$classes\$($kind[0])"
    Set-Default $k $kind[1]
    Set-Default "$k\DefaultIcon" ('"' + $icon + '",0')
    Set-Default "$k\shell\open\command" $open
}
# Leaf by name, for "Open with → Choose another app".
$app = "$classes\Applications\Leaf.exe"
New-Item -Path "$app\SupportedTypes" -Force | Out-Null
Set-ItemProperty -LiteralPath $app -Name "FriendlyAppName" -Value "Leaf"
foreach ($ext in $comics + $others) { Set-ItemProperty -LiteralPath "$app\SupportedTypes" -Name $ext -Value "" }
Set-Default "$app\DefaultIcon" ('"' + $icon + '",0')
Set-Default "$app\shell\open\command" $open
# "Open in Leaf" on folders: a folder of pages is a book.
Set-Default "$classes\Directory\shell\Leaf" "Open in Leaf"
Set-ItemProperty -LiteralPath "$classes\Directory\shell\Leaf" -Name "Icon" -Value ('"' + $icon + '",0')
Set-Default "$classes\Directory\shell\Leaf\command" $open

foreach ($ext in $comics + $others) {
    $progid = if ($comics -contains $ext) { "Leaf.Comic" } else { "Leaf.Book" }
    New-Item -Path "$classes\$ext\OpenWithProgids" -Force | Out-Null
    Set-ItemProperty -LiteralPath "$classes\$ext\OpenWithProgids" -Name $progid -Value ([byte[]]@()) -Type Binary
}
# Comics open in Leaf where nothing else opens them: no app you picked
# (Windows keeps that choice apart, and Leaf never touches it) and no app
# that set itself as the opener.
foreach ($ext in $comics) {
    $choice = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice"
    $picked = if (Test-Path -LiteralPath $choice) { [string](Get-Item -LiteralPath $choice).GetValue("ProgId") } else { "" }
    $current = Default "Registry::HKEY_CLASSES_ROOT\$ext"
    if ($picked -and $picked -ne "Leaf.Comic" -and $picked -ne "Applications\Leaf.exe") {
        Write-Output "$ext stays with $picked (Leaf is in its Open with list)"
    } elseif ($current -and $current -ne "Leaf.Comic") {
        Write-Output "$ext stays with $current (Leaf is in its Open with list)"
    } else {
        Set-Item -LiteralPath "$classes\$ext" -Value "Leaf.Comic"
    }
}
Refresh
Write-Output "Leaf is in the Open with lists. Run install.cmd again if you move this folder."
