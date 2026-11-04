import AppKit
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

guard CommandLine.arguments.count == 3 else { fail("用法：quit-macos.swift <bundle-id> <应用绝对路径>") }
let identifier = CommandLine.arguments[1]
let destination = URL(fileURLWithPath: CommandLine.arguments[2]).standardizedFileURL.resolvingSymlinksInPath()
// Other development copies are not ours to quit. The installer separately
// refuses replacement while any instance with the same bundle ID remains.
let applications = NSRunningApplication.runningApplications(withBundleIdentifier: identifier).filter {
    $0.bundleURL?.standardizedFileURL.resolvingSymlinksInPath() == destination
}
if !applications.isEmpty { print("正在请求 Velune 正常退出；如有未保存修改或运行中任务，请处理应用的退出确认。") }
for application in applications {
    guard application.terminate() else { fail("Velune 拒绝退出，未替换应用。") }
}
let deadline = Date().addingTimeInterval(30)
while applications.contains(where: { !$0.isTerminated }) {
    guard Date() < deadline else { fail("等待 Velune 退出超时，未替换应用；请正常退出后重试。") }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}
