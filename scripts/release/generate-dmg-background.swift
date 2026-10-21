#!/usr/bin/env swift
// Paint the DMG window's background: the "PwrRoute" wordmark on a dark pill,
// a subtitle, and an orange arrow from the app icon to the Applications link.
// Ported from PwrSnap's apps/desktop/scripts/generate-dmg-background.swift so
// the Pwr family's installers look alike.
//
// The PNG is committed (packaging/dmg/background.png): CI does not run Swift,
// and macos-dmg.sh reads it as a file. Rerun this after changing the layout
// or the artwork, and commit the result:
//
//   swift scripts/release/generate-dmg-background.swift [OUT.png]
//
// The font is the vendored Geist Bold in packaging/dmg/fonts (SIL OFL 1.1).

import AppKit
import CoreText
import Foundation

/// Register a font file from disk so its PostScript name becomes resolvable via
/// NSFont(name:size:). Returns the resolved PostScript name on success.
func registerFont(at url: URL) -> String? {
  guard FileManager.default.fileExists(atPath: url.path) else { return nil }
  var error: Unmanaged<CFError>?
  guard CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) else {
    if let err = error?.takeRetainedValue() {
      FileHandle.standardError.write(Data("Font register failed: \(err)\n".utf8))
    }
    return nil
  }
  guard
    let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor],
    let descriptor = descriptors.first,
    let psName = CTFontDescriptorCopyAttribute(descriptor, kCTFontNameAttribute) as? String
  else { return nil }
  return psName
}

// DMG window and icon layout — keep in sync with scripts/release/macos-dmg.sh
// (xtask's tests compare the two).
let width = 660
let height = 400
let iconSize = 112
let appIconX = 170
let applicationsX = 500
let iconY = 230

// Paths resolve from this file, so the script runs from any directory.
let root = URL(fileURLWithPath: #filePath)
  .deletingLastPathComponent()  // scripts/release
  .deletingLastPathComponent()  // scripts
  .deletingLastPathComponent()
let output = CommandLine.arguments.dropFirst().first.map { URL(fileURLWithPath: $0) }
  ?? root.appendingPathComponent("packaging/dmg/background.png")

let geistBoldURL = root.appendingPathComponent("packaging/dmg/fonts/Geist-Bold.ttf")
guard let geistBoldName = registerFont(at: geistBoldURL) else {
  // A system-font wordmark would not match the siblings; refuse to paint one.
  FileHandle.standardError.write(Data("generate-dmg-background: cannot load \(geistBoldURL.path)\n".utf8))
  exit(1)
}

struct Color {
  static let background = NSColor(calibratedRed: 0.965, green: 0.965, blue: 0.965, alpha: 1)
  // Pill behind the wordmark — near-black #0a0a0a, matching the app's titlebar
  // background. Pinned deviceRGB so the pixels land exactly; calibratedRGB
  // drifts lighter.
  static let pillBackground = NSColor(deviceRed: 10 / 255.0, green: 10 / 255.0, blue: 10 / 255.0, alpha: 1)
  static let text = NSColor(calibratedRed: 0.969, green: 0.953, blue: 0.922, alpha: 1)
  static let muted = NSColor(calibratedRed: 0.549, green: 0.522, blue: 0.478, alpha: 1)
  // Brand tangerine — design system: #ff8a1f (rgb 255, 138, 31). Pinned in
  // deviceRGB so the rendered pixels land on #ff8a1f exactly; calibratedRGB
  // drifts it to #ee894a, a third orange next to the wordmark and the icon.
  static let accent = NSColor(deviceRed: 255 / 255.0, green: 138 / 255.0, blue: 31 / 255.0, alpha: 1)
  static let arrowShaft = NSColor(deviceRed: 255 / 255.0, green: 138 / 255.0, blue: 31 / 255.0, alpha: 1)
}

func renderBackground() -> NSBitmapImageRep {
  guard let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: width,
    pixelsHigh: height,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
  ) else {
    fatalError("Unable to create bitmap")
  }
  bitmap.size = NSSize(width: CGFloat(width), height: CGFloat(height))

  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)

  let h = CGFloat(height)
  let w = CGFloat(width)

  // Light background
  Color.background.setFill()
  NSRect(x: 0, y: 0, width: w, height: h).fill()

  // Dark rounded pill behind logo and subtitle
  let pillWidth: CGFloat = 340
  let pillHeight: CGFloat = 100
  let pillX = (w - pillWidth) / 2
  let pillY = h - 120
  let pill = NSBezierPath(
    roundedRect: NSRect(x: pillX, y: pillY, width: pillWidth, height: pillHeight),
    xRadius: 16, yRadius: 16
  )
  Color.pillBackground.setFill()
  pill.fill()

  // Logo: "Pwr" + "Route" in Geist Bold, the family's display face.
  guard let logoFont = NSFont(name: geistBoldName, size: 40) else {
    fatalError("Geist Bold registered as \(geistBoldName) but does not resolve")
  }
  let logoY = pillY + pillHeight - 60
  let pwrSize = "Pwr".size(withAttributes: [.font: logoFont])
  let routeSize = "Route".size(withAttributes: [.font: logoFont])
  let logoX = (w - pwrSize.width - routeSize.width) / 2

  let textAttrs: [NSAttributedString.Key: Any] = [.font: logoFont, .foregroundColor: Color.text]
  let accentAttrs: [NSAttributedString.Key: Any] = [.font: logoFont, .foregroundColor: Color.accent]
  "Pwr".draw(at: NSPoint(x: logoX, y: logoY), withAttributes: textAttrs)
  "Route".draw(at: NSPoint(x: logoX + pwrSize.width, y: logoY), withAttributes: accentAttrs)

  // Subtitle: the design brief's line, "Every link to the right browser.",
  // in the family's two-noun form.
  let subtitleFont = NSFont.systemFont(ofSize: 11, weight: .medium)
  let subtitle = "links / browsers"
  let subtitleAttrs: [NSAttributedString.Key: Any] = [.font: subtitleFont, .foregroundColor: Color.muted]
  let subtitleSize = subtitle.size(withAttributes: subtitleAttrs)
  subtitle.draw(
    at: NSPoint(x: (w - subtitleSize.width) / 2, y: pillY + 14),
    withAttributes: subtitleAttrs
  )

  // Arrow — thick orange bar with chunky arrowhead
  let arrowStartX = CGFloat(appIconX + iconSize / 2 + 20)
  let arrowEndX = CGFloat(applicationsX - iconSize / 2 - 20)
  let arrowY = h - CGFloat(iconY)
  let shaftThickness: CGFloat = 18

  // Shaft — rounded rectangle
  let shaft = NSBezierPath(
    roundedRect: NSRect(
      x: arrowStartX,
      y: arrowY - shaftThickness / 2,
      width: arrowEndX - arrowStartX - 20,
      height: shaftThickness
    ),
    xRadius: shaftThickness / 2,
    yRadius: shaftThickness / 2
  )
  Color.arrowShaft.setFill()
  shaft.fill()

  // Arrowhead — solid orange triangle
  Color.accent.setFill()
  let headHeight: CGFloat = 48
  let headWidth: CGFloat = 32
  let arrowHead = NSBezierPath()
  arrowHead.move(to: NSPoint(x: arrowEndX, y: arrowY))
  arrowHead.line(to: NSPoint(x: arrowEndX - headWidth, y: arrowY + headHeight / 2))
  arrowHead.line(to: NSPoint(x: arrowEndX - headWidth, y: arrowY - headHeight / 2))
  arrowHead.close()
  arrowHead.fill()

  // "Drag to Applications" hint
  let instructionFont = NSFont.systemFont(ofSize: 12, weight: .medium)
  let instruction = "Drag to Applications"
  let instrColor = NSColor(calibratedRed: 0.45, green: 0.45, blue: 0.45, alpha: 1)
  let instrAttrs: [NSAttributedString.Key: Any] = [.font: instructionFont, .foregroundColor: instrColor]
  let instrSize = instruction.size(withAttributes: instrAttrs)
  instruction.draw(
    at: NSPoint(x: (w - instrSize.width) / 2, y: h - 366),
    withAttributes: instrAttrs
  )

  NSGraphicsContext.restoreGraphicsState()
  return bitmap
}

// Single 1x PNG — no multi-resolution TIFF.
let rep = renderBackground()

guard let pngData = rep.representation(using: .png, properties: [:]) else {
  fatalError("Unable to create PNG data")
}

try FileManager.default.createDirectory(at: output.deletingLastPathComponent(), withIntermediateDirectories: true)
try pngData.write(to: output)
print("Generated DMG background PNG (\(pngData.count / 1024) KB): \(output.path)")
