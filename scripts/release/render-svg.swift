// Rasterize an SVG with macOS's own renderer (NSImage reads SVG since macOS
// 11), one PNG per size, when rsvg-convert is not installed. Run by
// make-icns.sh; needs only the Xcode command line tools.
//
// usage: swift render-svg.swift SVG OUT_DIR SIZE...   writes OUT_DIR/SIZE.png
import AppKit
import Foundation

let args = CommandLine.arguments.dropFirst()
guard args.count >= 3, let svg = args.first, let outDir = args.dropFirst().first else {
    FileHandle.standardError.write(Data("usage: render-svg.swift SVG OUT_DIR SIZE...\n".utf8))
    exit(2)
}
guard let image = NSImage(contentsOfFile: svg) else {
    FileHandle.standardError.write(Data("render-svg: cannot read \(svg)\n".utf8))
    exit(1)
}
for sizeArg in args.dropFirst(2) {
    guard let size = Int(sizeArg), size > 0 else {
        FileHandle.standardError.write(Data("render-svg: bad size \(sizeArg)\n".utf8))
        exit(2)
    }
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)
    else { exit(1) }
    bitmap.size = NSSize(width: size, height: size)
    NSGraphicsContext.saveGraphicsState()
    let context = NSGraphicsContext(bitmapImageRep: bitmap)!
    NSGraphicsContext.current = context
    context.imageInterpolation = .high
    image.draw(in: NSRect(x: 0, y: 0, width: size, height: size),
               from: .zero, operation: .copy, fraction: 1)
    NSGraphicsContext.restoreGraphicsState()
    guard let png = bitmap.representation(using: .png, properties: [:]) else { exit(1) }
    let out = URL(fileURLWithPath: outDir).appendingPathComponent("\(size).png")
    do { try png.write(to: out) } catch {
        FileHandle.standardError.write(Data("render-svg: cannot write \(out.path): \(error)\n".utf8))
        exit(1)
    }
}
