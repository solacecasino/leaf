// Renders assets/icon.png (1024x1024) and assets/icon-256.png, the Leaf app
// icon: a leaf whose two halves open like the pages of a book, on the same
// dark body as Folio's icon.
// Run: swift scripts/make_icon.swift
import AppKit

let size: CGFloat = 1024

func render() -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    let rgb = CGColorSpaceCreateDeviceRGB()
    func color(_ r: CGFloat, _ g: CGFloat, _ b: CGFloat, _ a: CGFloat = 1) -> CGColor {
        NSColor(srgbRed: r, green: g, blue: b, alpha: a).cgColor
    }

    // Rounded-square body on Apple's icon grid (824pt body, 100pt inset).
    let body = CGRect(x: 100, y: 100, width: 824, height: 824)
    let bodyPath = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0, 0, 0, 0.35))
    ctx.addPath(bodyPath)
    ctx.setFillColor(color(0.10, 0.11, 0.12))
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(bodyPath)
    ctx.clip()
    let bodyGradient = CGGradient(colorsSpace: rgb, colors: [color(0.17, 0.20, 0.19), color(0.08, 0.09, 0.10)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(bodyGradient, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
    ctx.restoreGState()

    // The leaf, tilted, centred a little low so the stem has room.
    ctx.saveGState()
    ctx.translateBy(x: 512, y: 500)
    ctx.rotate(by: -0.6)
    let length: CGFloat = 330, width: CGFloat = 190

    // One half of the blade, from tip to base along the midrib.
    func half(_ side: CGFloat) -> CGPath {
        let p = CGMutablePath()
        p.move(to: CGPoint(x: 0, y: length))
        p.addCurve(to: CGPoint(x: 0, y: -length),
                   control1: CGPoint(x: side * width * 1.25, y: length * 0.55),
                   control2: CGPoint(x: side * width * 1.05, y: -length * 0.75))
        p.closeSubpath()
        return p
    }

    // Shadow under the whole leaf.
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -14), blur: 30, color: color(0, 0, 0, 0.5))
    ctx.addPath(half(-1)); ctx.addPath(half(1))
    ctx.setFillColor(color(0.31, 0.75, 0.45))
    ctx.fillPath()
    ctx.restoreGState()

    // Each half shaded on its own, like two facing pages.
    for (side, top, bottom) in [(CGFloat(-1), color(0.45, 0.86, 0.56), color(0.26, 0.66, 0.40)),
                                 (CGFloat(1), color(0.33, 0.74, 0.47), color(0.17, 0.52, 0.31))] {
        ctx.saveGState()
        ctx.addPath(half(side))
        ctx.clip()
        let g = CGGradient(colorsSpace: rgb, colors: [top, bottom] as CFArray, locations: [0, 1])!
        ctx.drawLinearGradient(g, start: CGPoint(x: 0, y: length), end: CGPoint(x: side * width, y: -length), options: [])
        // Veins: a few curves running from the midrib to the edge.
        ctx.setStrokeColor(color(1, 1, 1, 0.16))
        ctx.setLineWidth(9)
        ctx.setLineCap(.round)
        for i in 0..<5 {
            let y = length * 0.55 - CGFloat(i) * length * 0.27
            ctx.move(to: CGPoint(x: 0, y: y - 40))
            ctx.addQuadCurve(to: CGPoint(x: side * width * 0.95, y: y + 45), control: CGPoint(x: side * width * 0.45, y: y + 10))
        }
        ctx.strokePath()
        ctx.restoreGState()
    }

    // Midrib and stem, in the cream of a page.
    ctx.setStrokeColor(color(0.97, 0.95, 0.88))
    ctx.setLineCap(.round)
    ctx.setLineWidth(16)
    ctx.move(to: CGPoint(x: 0, y: length * 0.9))
    ctx.addLine(to: CGPoint(x: 0, y: -length))
    ctx.addQuadCurve(to: CGPoint(x: 38, y: -length - 120), control: CGPoint(x: 0, y: -length - 70))
    ctx.strokePath()
    ctx.restoreGState()

    NSGraphicsContext.restoreGraphicsState()
    return rep
}

func save(_ rep: NSBitmapImageRep, _ pixels: Int, _ path: String) {
    let out = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
    NSGraphicsContext.current!.imageInterpolation = .high
    rep.draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels))
    NSGraphicsContext.restoreGraphicsState()
    try! out.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

let rep = render()
try? FileManager.default.createDirectory(atPath: "assets", withIntermediateDirectories: true)
save(rep, 1024, "assets/icon.png")
save(rep, 256, "assets/icon-256.png")
print("wrote assets/icon.png and assets/icon-256.png")
