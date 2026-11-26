#if VELUNE_DEV_CHECKS
import Foundation

extension MessageEditingSession {
    /// Projection-only fixtures; never mutates a real editing operation.
    func prepareVisualCheckState(_ state: String) {
        guard store.isPreview, CommandLine.arguments.contains("--check-editor-visual") ||
                CommandLine.arguments.contains("--experience-message-editor-offscreen") else {
            FileHandle.standardError.write(Data("编辑器投影检查只允许合成预览入口。\n".utf8))
            exit(2)
        }
        if state.hasPrefix("busy") { setVisualCheckSubmitting() }
        if state.hasPrefix("failure") { setVisualCheckFailure("提交失败：连接已中断，编辑内容仍保留。请核对会话结果后重试。\n" + String(repeating: "远端暂时不可达；本次修改尚未得到确认。", count: 4)) }
        if state == "failure-target-changed" {
            setVisualCheckFailure("原生历史已执行回退，但尚未发送替换消息。编辑副本仍保留；请核对原会话后再决定下一步。\n" + String(repeating: "这是不能被目标变化提示遮住的真实失败详情。", count: 4))
        }
    }
}
#endif
