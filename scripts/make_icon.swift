// Renders assets/icon.png (1024x1024) and assets/icon-256.png: the leaf9000
// mark, an open book (two pages side by side) on the 9000 series' graphite
// body, the right-hand page cut on splice9000's slant into page gold, as
// the series' icons are.
// Run: swift scripts/make_icon.swift   (then python3 scripts/make_ico.py)
import AppKit

func render(_ size: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    let k = CGFloat(size) / 1024
    ctx.scaleBy(x: k, y: k)
    func color(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
        NSColor(srgbRed: CGFloat(hex >> 16 & 0xff) / 255, green: CGFloat(hex >> 8 & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255, alpha: a).cgColor
    }

    // Body on Apple's icon grid: 824 square, 100 inset.
    let body = CGPath(roundedRect: CGRect(x: 100, y: 100, width: 824, height: 824), cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0, 0.35))
    ctx.addPath(body)
    ctx.setFillColor(color(0x17181b))
    ctx.fillPath()
    ctx.restoreGState()
    ctx.saveGState()
    ctx.addPath(body)
    ctx.clip()
    let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [color(0x22242a), color(0x121316)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
    ctx.restoreGState()

    // Two pages, a gutter between them (y grows upward here).
    let (top, bottom): (CGFloat, CGFloat) = (790, 234)
    let (w, gutter, r): (CGFloat, CGFloat, CGFloat) = (268, 36, 40)
    let leftX = 512 - gutter / 2 - w, rightX = 512 + gutter / 2
    func page(_ x: CGFloat) -> CGPath {
        CGPath(roundedRect: CGRect(x: x, y: bottom, width: w, height: top - bottom), cornerWidth: r, cornerHeight: r, transform: nil)
    }
    ctx.addPath(page(leftX))
    ctx.setFillColor(color(0xe8e9eb))
    ctx.fillPath()

    // The right-hand page: light above, gold below a slanted cut.
    let cut: CGFloat = 430, slant: CGFloat = 30, gap: CGFloat = 22
    func piece(_ points: [CGPoint], _ fill: CGColor) {
        let p = CGMutablePath()
        p.addLines(between: points)
        p.closeSubpath()
        ctx.saveGState()
        ctx.addPath(page(rightX))
        ctx.clip()
        ctx.addPath(p)
        ctx.setFillColor(fill)
        ctx.fillPath()
        ctx.restoreGState()
    }
    let (x0, x1) = (rightX, rightX + w)
    // The cut falls toward the right, as grace9000's does.
    piece([CGPoint(x: x0, y: top), CGPoint(x: x1, y: top), CGPoint(x: x1, y: cut + gap / 2 - slant), CGPoint(x: x0, y: cut + gap / 2 + slant)], color(0xe8e9eb))
    piece([CGPoint(x: x0, y: cut - gap / 2 + slant), CGPoint(x: x1, y: cut - gap / 2 - slant), CGPoint(x: x1, y: bottom), CGPoint(x: x0, y: bottom)], color(0xffc23d))
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let dir = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "assets")
for (size, name) in [(1024, "icon.png"), (256, "icon-256.png"), (64, "icon-64.png"), (48, "icon-48.png"), (32, "icon-32.png"), (24, "icon-24.png"), (16, "icon-16.png")] {
    try! render(size).representation(using: .png, properties: [:])!.write(to: dir.appendingPathComponent(name))
}
print("wrote the icons in \(dir.path)")
