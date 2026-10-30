# Adds leaf9000 to this account's "Open with" lists, with its icon, for
# books and images, and "Open in leaf9000" to folders' right-click menu.
# Comic archives (.cbz, .cbr, .cb7, .cbt) open in leaf9000 by
# double-click, unless another app already opens them. Leaf's entries from
# before the rename now open leaf9000. Only your own account's settings
# change (HKEY_CURRENT_USER); nothing needs an administrator.
#
#   install.cmd            add leaf9000 (again, after moving this folder)
#   install.cmd --remove   take it out again
# install.cmd hands over its argument in LEAF_INSTALL: powershell -File
# would take "--remove" for a parameter name.
$Action = [string]$env:LEAF_INSTALL
$ErrorActionPreference = "Stop"

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$exe = Join-Path $here "leaf9000.exe"
$icon = Join-Path $here "leaf9000.ico"
$classes = "HKCU:\Software\Classes"
$comics = ".cbz", ".cbr", ".cb7", ".cbt"
$others = ".zip", ".epub", ".pdf", ".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp"
# The kinds of file keep Leaf's names (Leaf.Comic, Leaf.Book): a file you
# set to always open with Leaf names them, Windows won't let a script
# change that choice, and so it opens in leaf9000 now.
$ours = "Leaf.Comic", "Leaf.Book", "Applications\leaf9000.exe", "Directory\shell\leaf9000"
# Leaf's own entries from before the rename.
$leafs = "Applications\Leaf.exe", "Directory\shell\Leaf"

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
    foreach ($k in $ours + $leafs) { Remove-Item -LiteralPath "$classes\$k" -Recurse -Force -ErrorAction SilentlyContinue }
    foreach ($ext in $comics + $others) {
        Remove-ItemProperty -LiteralPath "$classes\$ext\OpenWithProgids" -Name "Leaf.Comic", "Leaf.Book" -ErrorAction SilentlyContinue
        if ((Default "$classes\$ext") -eq "Leaf.Comic") {
            $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Classes\$ext", $true)
            $k.DeleteValue("")
            $k.Close()
        }
    }
    Refresh
    Write-Output "leaf9000 is out of the Open with lists. (This folder is untouched; delete it to remove leaf9000.)"
    exit 0
}

if (-not (Test-Path -LiteralPath $exe)) { Write-Error "leaf9000.exe isn't next to install.cmd"; exit 1 }
foreach ($k in $leafs) { Remove-Item -LiteralPath "$classes\$k" -Recurse -Force -ErrorAction SilentlyContinue }
$open = '"' + $exe + '" "%1"'

# The two kinds of file leaf9000 opens, each with its icon and command.
foreach ($kind in @(("Leaf.Comic", "Comic book"), ("Leaf.Book", "Book or image"))) {
    $k = "$classes\$($kind[0])"
    Set-Default $k $kind[1]
    Set-Default "$k\DefaultIcon" ('"' + $icon + '",0')
    Set-Default "$k\shell\open\command" $open
}
# leaf9000 by name, for "Open with → Choose another app".
$app = "$classes\Applications\leaf9000.exe"
New-Item -Path "$app\SupportedTypes" -Force | Out-Null
Set-ItemProperty -LiteralPath $app -Name "FriendlyAppName" -Value "leaf9000"
foreach ($ext in $comics + $others) { Set-ItemProperty -LiteralPath "$app\SupportedTypes" -Name $ext -Value "" }
Set-Default "$app\DefaultIcon" ('"' + $icon + '",0')
Set-Default "$app\shell\open\command" $open
# "Open in leaf9000" on folders: a folder of pages is a book.
Set-Default "$classes\Directory\shell\leaf9000" "Open in leaf9000"
Set-ItemProperty -LiteralPath "$classes\Directory\shell\leaf9000" -Name "Icon" -Value ('"' + $icon + '",0')
Set-Default "$classes\Directory\shell\leaf9000\command" $open

foreach ($ext in $comics + $others) {
    $progid = if ($comics -contains $ext) { "Leaf.Comic" } else { "Leaf.Book" }
    New-Item -Path "$classes\$ext\OpenWithProgids" -Force | Out-Null
    Set-ItemProperty -LiteralPath "$classes\$ext\OpenWithProgids" -Name $progid -Value ([byte[]]@()) -Type Binary
}
# Comics open in leaf9000 where nothing else opens them: no app you picked
# (Windows keeps that choice apart, and leaf9000 never touches it) and no app
# that set itself as the opener.
foreach ($ext in $comics) {
    $choice = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\$ext\UserChoice"
    $picked = if (Test-Path -LiteralPath $choice) { [string](Get-Item -LiteralPath $choice).GetValue("ProgId") } else { "" }
    $current = Default "Registry::HKEY_CLASSES_ROOT\$ext"
    $mine = "Leaf.Comic", "Applications\Leaf.exe", "Applications\leaf9000.exe"
    if ($picked -and $mine -notcontains $picked) {
        Write-Output "$ext stays with $picked (leaf9000 is in its Open with list)"
    } elseif ($current -and $current -ne "Leaf.Comic") {
        Write-Output "$ext stays with $current (leaf9000 is in its Open with list)"
    } else {
        Set-Item -LiteralPath "$classes\$ext" -Value "Leaf.Comic"
    }
}
Refresh
Write-Output "leaf9000 is in the Open with lists. Run install.cmd again if you move this folder."
