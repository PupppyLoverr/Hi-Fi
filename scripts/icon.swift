// Renders the Hi-Fi app icon: an "H" whose crossbar is a sound wave.
// Usage: swift scripts/icon.swift Resources/icon-1024.png
import AppKit
import CoreGraphics

let out = CommandLine.arguments.dropFirst().first ?? "Resources/icon-1024.png"
let size = 1024
let cs = CGColorSpace(name: CGColorSpace.sRGB)!
let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
                    space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
ctx.translateBy(x: 0, y: CGFloat(size))
ctx.scaleBy(x: 1, y: -1)

func rgb(_ h: UInt32, _ a: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((h >> 16) & 0xff) / 255, green: CGFloat((h >> 8) & 0xff) / 255,
            blue: CGFloat(h & 0xff) / 255, alpha: a)
}

// macOS grid: 824pt squircle centred on the 1024 canvas.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let squircle = CGPath(roundedRect: tile, cornerWidth: 186, cornerHeight: 186, transform: nil)

ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: 14), blur: 36, color: rgb(0x1b2a4a, 0.22))
ctx.addPath(squircle)
ctx.setFillColor(rgb(0xffffff))
ctx.fillPath()
ctx.restoreGState()

ctx.saveGState()
ctx.addPath(squircle)
ctx.clip()
let bg = CGGradient(colorsSpace: cs, colors: [rgb(0xffffff), rgb(0xe3ecfa)] as CFArray,
                    locations: [0, 1])!
ctx.drawLinearGradient(bg, start: CGPoint(x: 512, y: 100), end: CGPoint(x: 512, y: 924), options: [])
// Soft sky glow, top-right.
let glow = CGGradient(colorsSpace: cs, colors: [rgb(0x9fd8ff, 0.55), rgb(0x9fd8ff, 0)] as CFArray,
                      locations: [0, 1])!
ctx.drawRadialGradient(glow, startCenter: CGPoint(x: 780, y: 230), startRadius: 0,
                       endCenter: CGPoint(x: 780, y: 230), endRadius: 520, options: [])
ctx.restoreGState()

// Hairline edge.
ctx.addPath(CGPath(roundedRect: tile.insetBy(dx: 1.5, dy: 1.5), cornerWidth: 185,
                   cornerHeight: 185, transform: nil))
ctx.setStrokeColor(rgb(0x0b1633, 0.08))
ctx.setLineWidth(3)
ctx.strokePath()

// Crossbar: one period of a sine wave between the stems.
let wave = CGMutablePath()
let x0 = 338.0, x1 = 686.0, amp = 58.0
for i in 0...120 {
    let t = Double(i) / 120
    let p = CGPoint(x: x0 + (x1 - x0) * t, y: 512 - amp * sin(t * 2 * .pi))
    if i == 0 { wave.move(to: p) } else { wave.addLine(to: p) }
}
ctx.saveGState()
ctx.addPath(wave.copy(strokingWithWidth: 60, lineCap: .round, lineJoin: .round, miterLimit: 10))
ctx.clip()
let accent = CGGradient(colorsSpace: cs, colors: [rgb(0x5b5cf6), rgb(0x38bdf8)] as CFArray,
                        locations: [0, 1])!
ctx.drawLinearGradient(accent, start: CGPoint(x: x0, y: 512), end: CGPoint(x: x1, y: 512),
                       options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
ctx.restoreGState()

// Stems.
let ink = rgb(0x1c1c22)
ctx.setLineCap(.round)
ctx.setLineWidth(84)
ctx.setStrokeColor(ink)
for x in [338.0, 686.0] {
    ctx.move(to: CGPoint(x: x, y: 318))
    ctx.addLine(to: CGPoint(x: x, y: 706))
}
ctx.strokePath()


let rep = NSBitmapImageRep(cgImage: ctx.makeImage()!)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
