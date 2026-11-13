import AppKit

let output = CommandLine.arguments[1]
var chunks = Data()
func uint32(_ value: Int) -> Data {
    var bigEndian = UInt32(value).bigEndian
    return withUnsafeBytes(of: &bigEndian) { Data($0) }
}
let types = [
    "16-1": "icp4", "16-2": "ic11", "32-1": "icp5", "32-2": "ic12",
    "128-1": "ic07", "128-2": "ic13", "256-1": "ic08", "256-2": "ic14",
    "512-1": "ic09", "512-2": "ic10"
]
for size in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let pixels = size * scale
        let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
                                  bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                  isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        let transform = NSAffineTransform()
        transform.scale(by: CGFloat(pixels) / 1024)
        transform.concat()
        NSColor(red: 24/255, green: 35/255, blue: 45/255, alpha: 1).setFill()
        NSBezierPath(roundedRect: NSRect(x: 50, y: 50, width: 924, height: 924), xRadius: 210, yRadius: 210).fill()
        let mint = NSColor(red: 106/255, green: 226/255, blue: 183/255, alpha: 1)
        mint.setFill()
        for (x, height) in [(230, 180), (370, 300), (510, 430)] {
            NSBezierPath(roundedRect: NSRect(x: x, y: 260, width: 92, height: height), xRadius: 24, yRadius: 24).fill()
        }
        NSColor.white.setStroke()
        let lens = NSBezierPath(ovalIn: NSRect(x: 555, y: 455, width: 220, height: 220))
        lens.lineWidth = 42
        lens.stroke()
        let handle = NSBezierPath()
        handle.move(to: NSPoint(x: 746, y: 480))
        handle.line(to: NSPoint(x: 829, y: 397))
        handle.lineWidth = 47
        handle.lineCapStyle = .round
        handle.stroke()
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        let url = URL(fileURLWithPath: "\(output)/icon_\(size)x\(size)\(suffix).png")
        let png = rep.representation(using: .png, properties: [:])!
        try png.write(to: url)
        chunks.append(types["\(size)-\(scale)"]!.data(using: .ascii)!)
        chunks.append(uint32(png.count + 8))
        chunks.append(png)
    }
}
var icon = "icns".data(using: .ascii)!
icon.append(uint32(chunks.count + 8))
icon.append(chunks)
try icon.write(to: URL(fileURLWithPath: CommandLine.arguments[2]))

