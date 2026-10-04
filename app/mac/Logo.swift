import SwiftUI

// Exact geometry and two-color fills from the supplied graphite optical masters.
// Small and medium retain their separate inner returns and inter-piece spacing.
struct VeluneMark: Shape {
    let smallMaster: Bool
    let rightPiece: Bool
    func path(in rect: CGRect) -> Path {
        let side = min(rect.width, rect.height)
        return (smallMaster ? small() : medium()).applying(CGAffineTransform(scaleX: side / 1024, y: side / 1024)).applying(CGAffineTransform(translationX: rect.midX - side / 2, y: rect.midY - side / 2))
    }
    private func small() -> Path {
        var p = Path()
        if rightPiece {
            p.move(to: CGPoint(x: 750, y: 238))
            p.addLine(to: CGPoint(x: 920, y: 238))
            p.addCurve(to: CGPoint(x: 942, y: 286), control1: CGPoint(x: 949, y: 238), control2: CGPoint(x: 958, y: 261))
            p.addLine(to: CGPoint(x: 626, y: 768))
            p.addCurve(to: CGPoint(x: 573, y: 769), control1: CGPoint(x: 612, y: 789), control2: CGPoint(x: 588, y: 790))
            p.addLine(to: CGPoint(x: 512, y: 689))
            p.addCurve(to: CGPoint(x: 529, y: 659), control1: CGPoint(x: 501, y: 672), control2: CGPoint(x: 510, y: 659))
            p.addLine(to: CGPoint(x: 555, y: 659))
            p.addCurve(to: CGPoint(x: 620, y: 541), control1: CGPoint(x: 624, y: 659), control2: CGPoint(x: 653, y: 597))
            p.addLine(to: CGPoint(x: 576, y: 477))
            p.addCurve(to: CGPoint(x: 577, y: 448), control1: CGPoint(x: 570, y: 467), control2: CGPoint(x: 571, y: 457))
            p.addLine(to: CGPoint(x: 708, y: 262))
            p.addCurve(to: CGPoint(x: 750, y: 238), control1: CGPoint(x: 720, y: 246), control2: CGPoint(x: 733, y: 238))
            p.closeSubpath()
        } else {
            p.move(to: CGPoint(x: 112, y: 212))
            p.addLine(to: CGPoint(x: 307, y: 212))
            p.addCurve(to: CGPoint(x: 355, y: 240), control1: CGPoint(x: 328, y: 212), control2: CGPoint(x: 344, y: 222))
            p.addLine(to: CGPoint(x: 549, y: 559))
            p.addCurve(to: CGPoint(x: 518, y: 611), control1: CGPoint(x: 561, y: 586), control2: CGPoint(x: 548, y: 611))
            p.addLine(to: CGPoint(x: 473, y: 611))
            p.addCurve(to: CGPoint(x: 440, y: 675), control1: CGPoint(x: 437, y: 611), control2: CGPoint(x: 420, y: 642))
            p.addLine(to: CGPoint(x: 517, y: 799))
            p.addCurve(to: CGPoint(x: 509, y: 812), control1: CGPoint(x: 522, y: 807), control2: CGPoint(x: 518, y: 812))
            p.addLine(to: CGPoint(x: 428, y: 812))
            p.addCurve(to: CGPoint(x: 383, y: 788), control1: CGPoint(x: 408, y: 812), control2: CGPoint(x: 393, y: 804))
            p.addLine(to: CGPoint(x: 80, y: 265))
            p.addCurve(to: CGPoint(x: 112, y: 212), control1: CGPoint(x: 62, y: 234), control2: CGPoint(x: 87, y: 212))
            p.closeSubpath()
        }
        return p
    }
    private func medium() -> Path {
        var p = Path()
        if rightPiece {
            p.move(to: CGPoint(x: 740, y: 241))
            p.addLine(to: CGPoint(x: 910, y: 241))
            p.addCurve(to: CGPoint(x: 932, y: 289), control1: CGPoint(x: 939, y: 241), control2: CGPoint(x: 948, y: 264))
            p.addLine(to: CGPoint(x: 616, y: 771))
            p.addCurve(to: CGPoint(x: 563, y: 772), control1: CGPoint(x: 602, y: 792), control2: CGPoint(x: 578, y: 793))
            p.addLine(to: CGPoint(x: 497, y: 679))
            p.addCurve(to: CGPoint(x: 513, y: 651), control1: CGPoint(x: 486, y: 666), control2: CGPoint(x: 495, y: 651))
            p.addLine(to: CGPoint(x: 545, y: 651))
            p.addCurve(to: CGPoint(x: 606, y: 545), control1: CGPoint(x: 607, y: 651), control2: CGPoint(x: 637, y: 598))
            p.addLine(to: CGPoint(x: 566, y: 480))
            p.addCurve(to: CGPoint(x: 567, y: 451), control1: CGPoint(x: 560, y: 470), control2: CGPoint(x: 561, y: 460))
            p.addLine(to: CGPoint(x: 698, y: 265))
            p.addCurve(to: CGPoint(x: 740, y: 241), control1: CGPoint(x: 710, y: 249), control2: CGPoint(x: 723, y: 241))
            p.closeSubpath()
        } else {
            p.move(to: CGPoint(x: 122, y: 212))
            p.addLine(to: CGPoint(x: 317, y: 212))
            p.addCurve(to: CGPoint(x: 365, y: 240), control1: CGPoint(x: 338, y: 212), control2: CGPoint(x: 354, y: 222))
            p.addLine(to: CGPoint(x: 559, y: 559))
            p.addCurve(to: CGPoint(x: 529, y: 613), control1: CGPoint(x: 574, y: 585), control2: CGPoint(x: 561, y: 613))
            p.addLine(to: CGPoint(x: 482, y: 613))
            p.addCurve(to: CGPoint(x: 448, y: 675), control1: CGPoint(x: 445, y: 613), control2: CGPoint(x: 428, y: 643))
            p.addLine(to: CGPoint(x: 527, y: 799))
            p.addCurve(to: CGPoint(x: 519, y: 812), control1: CGPoint(x: 532, y: 807), control2: CGPoint(x: 528, y: 812))
            p.addLine(to: CGPoint(x: 438, y: 812))
            p.addCurve(to: CGPoint(x: 393, y: 788), control1: CGPoint(x: 418, y: 812), control2: CGPoint(x: 403, y: 804))
            p.addLine(to: CGPoint(x: 90, y: 265))
            p.addCurve(to: CGPoint(x: 122, y: 212), control1: CGPoint(x: 72, y: 234), control2: CGPoint(x: 97, y: 212))
            p.closeSubpath()
        }
        return p
    }
}

struct VeluneLogo: View {
    let size: CGFloat
    var body: some View {
        ZStack {
            VeluneMark(smallMaster: size < 32, rightPiece: false).fill(VeluneTheme.graphiteLeft)
            VeluneMark(smallMaster: size < 32, rightPiece: true).fill(VeluneTheme.graphiteRight)
        }.frame(width: size, height: size).accessibilityLabel("Velune")
    }
}
