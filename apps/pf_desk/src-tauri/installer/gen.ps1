# Regenerates the installer BMPs from ../app-icon.png. Run: pwsh installer/gen.ps1
Add-Type -AssemblyName System.Drawing
$here = $PSScriptRoot
$icon = [Drawing.Image]::FromFile((Join-Path $here '../app-icon.png'))
$ink = (New-Object Drawing.Bitmap $icon).GetPixel(60, 60)  # icon background, so the logo blends in
$amber = [Drawing.Color]::FromArgb(245, 158, 11)  # --color-primary
$mute = [Drawing.Color]::FromArgb(163, 163, 173)  # --color-muted
$src = New-Object Drawing.Rectangle 190, 190, 640, 640  # logo region, crops the icon's padding

function New-Bmp($name, $w, $h, $paint) {
  $b = New-Object Drawing.Bitmap $w, $h
  $g = [Drawing.Graphics]::FromImage($b)
  $g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic';
  $g.TextRenderingHint = 'AntiAliasGridFit'
  & $paint $g $w $h
  $g.Dispose()
  $b.Save((Join-Path $here $name), [Drawing.Imaging.ImageFormat]::Bmp); $b.Dispose()
}
function Logo($g, $x, $y, $s) { $g.DrawImage($icon, (New-Object Drawing.Rectangle $x, $y, $s, $s), $src, 'Pixel') }
function Brand($g, $x, $y, $size, $color) {
  $f = New-Object Drawing.Font 'Segoe UI Semibold', $size, 'Regular', 'Pixel'
  $g.DrawString('PARC FERMÉ', $f, (New-Object Drawing.SolidBrush $color), $x, $y)
}
$bar = { param($g, $x, $y, $w, $h) $g.FillRectangle((New-Object Drawing.SolidBrush $amber), $x, $y, $w, $h) }
$white = [Drawing.Color]::White

# Dark sidebar panel (NSIS welcome/finish; left panel of the MSI dialog)
$side = { param($g, $w, $h)
  $g.Clear($ink); Logo $g 12 24 140
  Brand $g 18 176 17 $white
  & $bar $g 18 206 36 3
  $f = New-Object Drawing.Font 'Segoe UI', 11, 'Regular', 'Pixel'
  $g.DrawString("Setups, telemetry`nand sync for sim racers.", $f, (New-Object Drawing.SolidBrush $mute), 18, 218)
  & $bar $g 0 ($h - 6) $w 6 }

New-Bmp 'nsis-sidebar.bmp' 164 314 $side
New-Bmp 'nsis-header.bmp' 150 57 { param($g, $w, $h) $g.Clear($ink); Logo $g 8 4 49; Brand $g 62 18 14 $white; & $bar $g 0 ($h - 3) $w 3 }

# MSI: text is drawn over these in black, so only the left panel / right edge may be dark
New-Bmp 'wix-dialog.bmp' 493 312 { param($g, $w, $h) $g.Clear($white); $g.SetClip((New-Object Drawing.Rectangle 0, 0, 164, $h)); & $side $g 164 $h $side $g 164 $h; $g.ResetClip() }
New-Bmp 'wix-banner.bmp' 493 58 { param($g, $w, $h) $g.Clear($white); Logo $g ($w - 54) 2 54; & $bar $g 0 ($h - 3) $w 3 }
