//! Small bounded observations shared by native background-task adapters.
use crate::conversation::*;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(crate) fn text(value: &str, limit: usize) -> (String, bool) {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), end < value.len())
}
pub(crate) fn status(value: &str) -> BackgroundTaskStatus {
    match value {
        "running" | "in_progress" | "pending" => BackgroundTaskStatus::Running,
        "stopping" => BackgroundTaskStatus::Stopping,
        "completed" | "committed" => BackgroundTaskStatus::Completed,
        "failed" | "error" => BackgroundTaskStatus::Failed,
        "killed" | "stopped" | "cancelled" => BackgroundTaskStatus::Cancelled,
        _ => BackgroundTaskStatus::Unknown,
    }
}

#[derive(Default)]
pub(crate) struct Reads {
    next: u64,
    entries: VecDeque<(BackgroundTaskOutput, Instant)>,
}
impl Reads {
    pub(crate) fn pending(&self) -> bool {
        self.entries.iter().any(|(entry, started)| {
            entry.status == BackgroundTaskOutputStatus::Checking
                && started.elapsed() < Duration::from_secs(10)
        })
    }
    pub(crate) fn begin(&mut self, task: &str) -> BackgroundTaskOutput {
        self.next = self.next.wrapping_add(1);
        let output = BackgroundTaskOutput {
            request_id: format!("background-{}", self.next),
            task_id: task.into(),
            status: BackgroundTaskOutputStatus::Checking,
            text: String::new(),
            truncated: false,
            issue: None,
        };
        while self.entries.len() >= 8 {
            self.entries.pop_front();
        }
        self.entries.push_back((output.clone(), Instant::now()));
        output
    }
    pub(crate) fn read(&mut self, task: &str, request: &str) -> Option<BackgroundTaskOutput> {
        let (value, started) = self
            .entries
            .iter_mut()
            .find(|(entry, _)| entry.task_id == task && entry.request_id == request)?;
        if value.status == BackgroundTaskOutputStatus::Checking
            && started.elapsed() > Duration::from_secs(10)
        {
            value.status = BackgroundTaskOutputStatus::Failed;
            value.issue = Some("任务输出读取超时，可以重试；任务未被中断。".into());
        }
        Some(value.clone())
    }
    pub(crate) fn finish(
        &mut self,
        request: &str,
        task: &str,
        value: &str,
        truncated: bool,
        issue: Option<String>,
    ) {
        let Some((output, _)) = self.entries.iter_mut().find(|(entry, _)| {
            entry.task_id == task
                && entry.request_id == request
                && entry.status == BackgroundTaskOutputStatus::Checking
        }) else {
            return;
        };
        let (value, bounded) = text(value, 65_536);
        output.text = value;
        output.truncated = truncated || bounded;
        output.status = if issue.is_some() {
            BackgroundTaskOutputStatus::Failed
        } else {
            BackgroundTaskOutputStatus::Ready
        };
        output.issue = issue.map(|issue| text(&issue, 512).0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_tasks_output_cache_is_bounded_and_task_scoped() {
        let mut reads = Reads::default();
        let first = reads.begin("first");
        reads.finish(
            &first.request_id,
            "different",
            "must not attach",
            false,
            None,
        );
        assert_eq!(
            reads.read("first", &first.request_id).unwrap().status,
            BackgroundTaskOutputStatus::Checking
        );
        reads.finish(
            &first.request_id,
            "first",
            &"中文".repeat(20_000),
            false,
            None,
        );
        let output = reads.read("first", &first.request_id).unwrap();
        assert!(output.truncated && output.text.len() <= 65_536);
        for index in 0..8 {
            reads.begin(&index.to_string());
        }
        assert!(reads.read("first", &first.request_id).is_none());
        assert_eq!(reads.entries.len(), 8);
        let (last, started) = reads.entries.back_mut().unwrap();
        let (task, request) = (last.task_id.clone(), last.request_id.clone());
        *started = Instant::now() - Duration::from_secs(11);
        assert_eq!(
            reads.read(&task, &request).unwrap().status,
            BackgroundTaskOutputStatus::Failed
        );
    }
}
