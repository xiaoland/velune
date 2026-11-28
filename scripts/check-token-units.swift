import Foundation

@main
struct TokenCountChecks {
    static func main() {
        let examples: [(UInt32, String)] = [(0, "0"), (1, "1"), (999, "999"), (1000, "1K"), (8192, "8.192K"), (272000, "272K"), (999999, "999.999K"), (1000000, "1M"), (1500000, "1.5M"), (100000000, "100M"), (UInt32.max, "4.294967295B")]
        for (count, text) in examples {
            precondition(TokenCount.format(count) == text)
            if count > 0 { precondition(TokenCount.parse(text) == count) }
        }
        for (text, count): (String, UInt32) in [(" 1.5m ", 1500000), ("1 k", 1000), ("1,000", 1000), ("100,000,000", 100000000), ("0.001K", 1), ("1.0000", 1)] {
            precondition(TokenCount.parse(text) == count, text)
        }
        for text in ["", "0", "-1", "+1", "1.2", "0.0001K", "1e3", "1KiB", "1,5M", "1,00", "NaN", "∞", "4.294967296B", "4294967296", "1.00000000000000000001M"] {
            precondition(TokenCount.parse(text) == nil, text)
        }
        // Padding and trimming depend on the first and last nonzero decimal places.
        for scale: UInt32 in [1_000, 1_000_000, 1_000_000_000] {
            var firstPlace: UInt32 = 1
            while firstPlace < scale {
                var lastPlace: UInt32 = 1
                while lastPlace <= firstPlace {
                    let count = scale + 7 * firstPlace + 2 * lastPlace
                    precondition(TokenCount.parse(TokenCount.format(count)) == count, "\(count)")
                    lastPlace *= 10
                }
                firstPlace *= 10
            }
        }
        precondition(TokenCount.format(UInt64.max) == "18446744073.709551615B")
        precondition(TokenCount.validLimits(context: " ", output: ""))
        precondition(TokenCount.validLimits(context: "1M", output: "32K"))
        precondition(!TokenCount.validLimits(context: "1K", output: "1M"))
        precondition(!TokenCount.validLimits(context: "wrong", output: ""))
        precondition(TokenCount.rate(1500) == "1.5K")
        for (count, text): (UInt64, String) in [(0, "0"), (999, "999"), (1000, "1K"), (8192, "8.19K"), (52143, "52.1K"), (521433, "521K"),
                                                (999_499, "999K"), (999_999, "1M"), (1_050_000, "1.05M"), (12_345_678, "12.3M"), (4_294_967_295, "4.29B")] {
            precondition(TokenCount.approximate(count) == text, "\(count): \(TokenCount.approximate(count))")
        }
        print("Swift token 单位与无损回读检查通过")
    }
}
