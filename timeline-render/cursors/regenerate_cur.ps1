# Regenera gain.cur e hand.cur no formato CUR clássico (DIB + AND/XOR masks),
# confiável com LoadImageW(IMAGE_CURSOR). Depende de System.Drawing.
# Uso: powershell -ExecutionPolicy Bypass -File regenerate_cur.ps1

Add-Type -AssemblyName System.Drawing

function Build-Cursor([string] $outPath, [int] $hotX, [int] $hotY, [System.Drawing.Bitmap] $bmp) {
    $S = 32
    # Extrai pixels em BGRA, linha de baixo para cima (ordem DIB).
    $xor = New-Object System.Collections.Generic.List[byte]
    $bytesPerRow = $S * 4
    $pixelRows = @()
    for ($y = 0; $y -lt $S; $y++) {
        $row = New-Object byte[] $bytesPerRow
        $srcY = $S - 1 - $y   # DIB armazena de baixo para cima
        for ($x = 0; $x -lt $S; $x++) {
            $c = $bmp.GetPixel($x, $srcY)
            $row[$x*4 + 0] = $c.B   # B
            $row[$x*4 + 1] = $c.G   # G
            $row[$x*4 + 2] = $c.R   # R
            $row[$x*4 + 3] = $c.A   # A
        }
        $pixelRows += ,$row
    }

    # XOR mask (32bpp BGRA), com linhas de 0..31 como extraídas.
    foreach ($row in $pixelRows) { $xor.AddRange($row) }

    # AND mask: 1bpp, 1 = transparente, 0 = opaco. 32 px / 8 = 4 bytes por linha.
    $andBytesPerRow = 4
    $and = New-Object System.Collections.Generic.List[byte]
    $andRows = @()
    for ($y = 0; $y -lt $S; $y++) {
        $rowBytes = New-Object byte[] $andBytesPerRow
        for ($x = 0; $x -lt $S; $x++) {
            $alpha = $bmp.GetPixel($x, ($S - 1 - $y)).A
            if ($alpha -eq 0) {
                $rowBytes[[int][math]::Floor($x/8)] = $rowBytes[[int][math]::Floor($x/8)] -bor (0x80 -shr ($x % 8))
            }
        }
        $andRows += ,$rowBytes
    }
    foreach ($row in $andRows) { $and.AddRange($row) }

    # --- Monta o arquivo CUR ---
    $iconDirSize = 6 + 16
    $biSize = 40
    $xorSize = $bytesPerRow * $S      # 4096
    $andSize = $andBytesPerRow * $S   # 128
    $dataSize = $biSize + $xorSize + $andSize

    $fs = [System.IO.File]::Create($outPath)
    try {
        $bw = New-Object System.IO.BinaryWriter($fs)
        try {
            # ICONDIR
            $bw.Write([uint16]0)              # reserved
            $bw.Write([uint16]2)              # type = CUR
            $bw.Write([uint16]1)              # count
            # ICONDIRENTRY
            $bw.Write([byte]$S)               # width
            $bw.Write([byte]$S)               # height
            $bw.Write([byte]0)                # colors
            $bw.Write([byte]0)                # reserved
            $bw.Write([uint16]$hotX)          # x hotspot
            $bw.Write([uint16]$hotY)          # y hotspot
            $bw.Write([uint32]$dataSize)      # dwBytesInRes
            $bw.Write([uint32]$iconDirSize)   # dwImageOffset
            # BITMAPINFOHEADER
            $bw.Write([uint32]$biSize)
            $bw.Write([int32]$S)              # biWidth
            $bw.Write([int32]($S * 2))        # biHeight (XOR + AND mask)
            $bw.Write([uint16]1)              # biPlanes
            $bw.Write([uint16]32)             # biBitCount
            $bw.Write([uint32]0)              # biCompression = BI_RGB
            $bw.Write([uint32]0)
            $bw.Write([int32]0)               # biXPelsPerMeter
            $bw.Write([int32]0)               # biYPelsPerMeter
            $bw.Write([uint32]0)              # biClrUsed
            $bw.Write([uint32]0)              # biClrImportant
            # XOR mask
            $xorArray = $xor.ToArray()
            $bw.Write($xorArray)
            # AND mask
            $andArray = $and.ToArray()
            $bw.Write($andArray)
            $bw.Flush()
        } finally {
            $bw.Dispose()
        }
    } finally {
        $fs.Dispose()
    }
    Write-Output "Escrito: $outPath ($((Get-Item $outPath).Length) bytes)"
}

# ---------------------------------------------------------------
# 1) gain.cur — seta de ganho amarela com contorno preto
# ---------------------------------------------------------------
$S = 32
$gain = New-Object System.Drawing.Bitmap($S, $S, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($gain)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::Transparent)

$cx = 16.0
$tipY = 3.0
$baseTop = 10.0
$tri = [System.Drawing.PointF[]]@(
    (New-Object System.Drawing.PointF($cx, $tipY)),
    (New-Object System.Drawing.PointF(($cx - 9), ($baseTop - 2))),
    (New-Object System.Drawing.PointF(($cx + 9), ($baseTop - 2)))
)
# Contorno preto do triângulo.
$g.DrawPolygon((New-Object System.Drawing.Pen([System.Drawing.Color]::Black, 3)), $tri)
# Preenchimento amarelo.
$g.FillPolygon((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Yellow)), $tri)
# Haste vertical com contorno preto e miolo amarelo.
$g.DrawLine((New-Object System.Drawing.Pen([System.Drawing.Color]::Black, 9)), $cx, $baseTop + 3, $cx, 30)
$g.DrawLine((New-Object System.Drawing.Pen([System.Drawing.Color]::Yellow, 5)), $cx, $baseTop + 3, $cx, 30)
$g.Dispose()
Build-Cursor 'D:\Dev\Porjetos\RuanaStudio\timeline-render\cursors\gain.cur' 15 11 $gain
$gain.Dispose()

# ---------------------------------------------------------------
# 2) hand.cur — mão aberta branca com contorno preto
# ---------------------------------------------------------------
$S = 32
$hand = New-Object System.Drawing.Bitmap($S, $S, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($hand)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::Transparent)

$fill = [System.Drawing.Color]::White
$brush = New-Object System.Drawing.SolidBrush($fill)
$darkPen = New-Object System.Drawing.Pen([System.Drawing.Color]::Black, 2.2)

$fingers = @(
    @{ x = 4;  y = 3;  w = 5; h = 9 },
    @{ x = 10; y = 2;  w = 5; h = 11 },
    @{ x = 16; y = 1;  w = 5; h = 12 },
    @{ x = 22; y = 3;  w = 5; h = 10 },
    @{ x = 26; y = 7;  w = 4; h = 8 }
)
foreach ($f in $fingers) {
    $r = New-Object System.Drawing.Rectangle($f.x, $f.y, $f.w, $f.h)
    $g.FillRectangle($brush, $r)
    $g.DrawRectangle($darkPen, $r)
}
$palma = New-Object System.Drawing.Rectangle(10, 11, 22, 16)
$g.FillEllipse($brush, $palma)
$g.DrawEllipse($darkPen, $palma)
$pulso = New-Object System.Drawing.Rectangle(13, 22, 16, 9)
$g.FillRectangle($brush, $pulso)
$g.DrawRectangle($darkPen, $pulso)
$g.Dispose()
Build-Cursor 'D:\Dev\Porjetos\RuanaStudio\timeline-render\cursors\hand.cur' 14 16 $hand
$hand.Dispose()