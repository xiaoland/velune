import AppKit
import Foundation

// Render the supplied optical masters without changing their path geometry.
final class LogoReader: NSObject, XMLParserDelegate {
    var paths: [(String, String)] = []
    func parser(_ parser: XMLParser, didStartElement name: String, namespaceURI: String?, qualifiedName: String?, attributes: [String: String]) {
        if name == "path", let data = attributes["d"], let fill = attributes["fill"] {
            paths.append((data, fill))
        }
    }
}

func path(_ data: String) throws -> NSBezierPath {
    let pattern = try NSRegularExpression(pattern: "[MLCZ]|-?[0-9]+(?:\\.[0-9]+)?")
    let tokens = pattern.matches(in: data, range: NSRange(data.startIndex..., in: data)).map { String(data[Range($0.range, in: data)!]) }
    let result = NSBezierPath()
    var index = 0
    func point() throws -> NSPoint {
        guard index + 1 < tokens.count, let x = Double(tokens[index]), let y = Double(tokens[index + 1]) else {
            throw NSError(domain: "VeluneLogo", code: 1)
        }
        index += 2
        return NSPoint(x: x, y: y)
    }
    while index < tokens.count {
        let command = tokens[index]; index += 1
        switch command {
        case "M": result.move(to: try point())
        case "L": result.line(to: try point())
        case "C":
            let first = try point(), second = try point(), end = try point()
            result.curve(to: end, controlPoint1: first, controlPoint2: second)
        case "Z": result.close()
        default: throw NSError(domain: "VeluneLogo", code: 2)
        }
    }
    return result
}

guard CommandLine.arguments.count == 3 else { fatalError("Expected brand directory and iconset output") }
let source = URL(fileURLWithPath: CommandLine.arguments[1])
let destination = URL(fileURLWithPath: CommandLine.arguments[2])
try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    let master = points < 32 ? "small" : points < 96 ? "medium" : "large"
    let parser = XMLParser(data: try Data(contentsOf: source.appendingPathComponent("velune-\(master)-graphite.svg")))
    let reader = LogoReader(); parser.delegate = reader
    guard parser.parse(), reader.paths.count == 2 else { fatalError("Invalid optical logo master") }
    for scale in [1, 2] {
        let size = points * scale
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        let context = NSGraphicsContext(bitmapImageRep: bitmap)!
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = context
        context.cgContext.setShouldAntialias(true)
        let dimension = CGFloat(size)
        NSColor(srgbRed: 0.97, green: 0.97, blue: 0.985, alpha: 1).setFill()
        NSBezierPath(roundedRect: NSRect(x: dimension * 0.035, y: dimension * 0.035, width: dimension * 0.93, height: dimension * 0.93), xRadius: dimension * 0.21, yRadius: dimension * 0.21).fill()
        let margin = dimension * 0.10
        context.cgContext.translateBy(x: margin, y: dimension - margin)
        context.cgContext.scaleBy(x: (dimension - margin * 2) / 1024, y: -(dimension - margin * 2) / 1024)
        for (data, fill) in reader.paths {
            guard let hex = UInt32(fill.dropFirst(), radix: 16) else { fatalError("Invalid logo color") }
            NSColor(srgbRed: CGFloat((hex >> 16) & 255) / 255, green: CGFloat((hex >> 8) & 255) / 255, blue: CGFloat(hex & 255) / 255, alpha: 1).setFill()
            try path(data).fill()
        }
        NSGraphicsContext.restoreGraphicsState()
        let suffix = scale == 2 ? "@2x" : ""
        try bitmap.representation(using: .png, properties: [:])!.write(to: destination.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}
