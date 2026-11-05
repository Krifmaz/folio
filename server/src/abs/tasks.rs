//! The port of the original's task list (`server/managers/TaskManager.js`, `objects/Task.js`): the long jobs the
//! apps show in their task panel (scans, metadata embedding, backups, downloads). A job is added when it starts
//! (`task_started` to every app) and removed when it finishes (`task_finished`). `GET /api/tasks` lists the
//! running ones.

use crate::abs::models;
use crate::abs::socket::Sockets;
use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::Mutex;

/// A text the apps show, with the translation key and values they use instead when they have them
/// (`{text, key, subs}` in the original).
#[derive(Clone, Debug, Default)]
pub struct TaskText {
    /// The English text.
    pub text: String,
    /// The translation key, if any.
    pub key: Option<String>,
    /// The values for the translation, if any.
    pub subs: Option<Vec<Value>>,
}

impl TaskText {
    /// A plain text without a translation key.
    pub fn plain(text: &str) -> TaskText {
        TaskText { text: text.to_string(), key: None, subs: None }
    }
}

/// One job as the apps see it (`Task.toJSON`).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// The task's id (a UUID).
    pub id: String,
    /// What it does, e.g. `embed-metadata`, `encode-m4b`, `library-scan`.
    pub action: String,
    /// What it works on, e.g. `{"libraryItemId": …}`.
    pub data: Map<String, Value>,
    /// Its title.
    pub title: Option<String>,
    /// The title's translation key.
    pub title_key: Option<String>,
    /// The title's translation values.
    pub title_subs: Option<Vec<Value>>,
    /// What it is doing.
    pub description: Option<String>,
    /// The description's translation key.
    pub description_key: Option<String>,
    /// The description's translation values.
    pub description_subs: Option<Vec<Value>>,
    /// Why it failed.
    pub error: Option<String>,
    /// The error's translation key.
    pub error_key: Option<String>,
    /// The error's translation values.
    pub error_subs: Option<Vec<Value>>,
    /// Whether the apps tell the user when it succeeds.
    pub show_success: bool,
    /// Whether it failed.
    pub is_failed: bool,
    /// Whether it is over.
    pub is_finished: bool,
    /// When it started (milliseconds).
    pub started_at: Option<i64>,
    /// When it ended (milliseconds).
    pub finished_at: Option<i64>,
}

impl Task {
    /// A new task (`setData`), started now.
    pub fn new(action: &str, title: TaskText, description: Option<TaskText>, show_success: bool, data: Map<String, Value>) -> Task {
        let description = description.filter(|d| !d.text.is_empty());
        Task {
            id: models::new_id(),
            action: action.to_string(),
            data,
            title: Some(title.text),
            title_key: title.key,
            title_subs: title.subs,
            description_key: description.as_ref().and_then(|d| d.key.clone()),
            description_subs: description.as_ref().and_then(|d| d.subs.clone()),
            description: description.map(|d| d.text),
            show_success,
            started_at: Some(models::now_ms()),
            ..Task::default()
        }
    }

    /// Marks the task failed (`setFailed`), which also ends it.
    pub fn set_failed(&mut self, message: TaskText) {
        self.error = Some(message.text);
        self.error_key = message.key;
        self.error_subs = message.subs;
        self.is_failed = true;
        self.set_finished(None, false);
    }

    /// Marks the task over (`setFinished`), with a new description or none when asked.
    pub fn set_finished(&mut self, description: Option<TaskText>, clear_description: bool) {
        if let Some(d) = description.filter(|d| !d.text.is_empty()) {
            self.description = Some(d.text);
            self.description_key = d.key;
            self.description_subs = d.subs;
        } else if clear_description {
            self.description = None;
            self.description_key = None;
            self.description_subs = None;
        }
        self.is_finished = true;
        self.finished_at = Some(models::now_ms());
    }
}

/// The running tasks, shared by every part of the port.
#[derive(Default)]
pub struct TaskManager {
    /// The tasks, oldest first.
    tasks: Mutex<Vec<Task>>,
}

impl TaskManager {
    /// The running tasks (`tasks`).
    pub fn tasks(&self) -> Vec<Task> {
        self.tasks.lock().map(|t| t.clone()).unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Adds a started task and tells every app (`addTask`).
    pub fn add(&self, sockets: &Sockets, task: Task) {
        sockets.emitter("task_started", &task, None);
        match self.tasks.lock() {
            Ok(mut t) => t.push(task),
            Err(e) => e.into_inner().push(task),
        }
    }

    /// Removes a finished task and tells every app (`taskFinished`); a task not in the list is ignored.
    pub fn finished(&self, sockets: &Sockets, task: &Task) {
        let removed = {
            let mut list = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            let before = list.len();
            list.retain(|t| t.id != task.id);
            list.len() != before
        };
        if removed {
            sockets.emitter("task_finished", task, None);
        }
    }

    /// Makes, adds and returns a task (`createAndAddTask`).
    pub fn create_and_add(
        &self,
        sockets: &Sockets,
        action: &str,
        title: TaskText,
        description: Option<TaskText>,
        show_success: bool,
        data: Map<String, Value>,
    ) -> Task {
        let task = Task::new(action, title, description, show_success, data);
        self.add(sockets, task.clone());
        task
    }

    /// Tells every app about a task that failed before it could start, without listing it
    /// (`createAndEmitFailedTask`).
    pub fn emit_failed(&self, sockets: &Sockets, action: &str, title: TaskText, description: Option<TaskText>, error: TaskText) -> Task {
        let mut task = Task::new(action, title, description, false, Map::new());
        task.set_failed(error);
        sockets.emitter("task_started", &task, None);
        task
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_are_listed_while_they_run() {
        let sockets = Sockets::default();
        let m = TaskManager::default();
        let mut t = m.create_and_add(&sockets, "library-scan", TaskText::plain("Scanning"), None, true, Map::new());
        assert_eq!(m.tasks().len(), 1);
        let j = serde_json::to_value(&t).unwrap();
        assert_eq!(j["action"], "library-scan");
        assert_eq!(j["titleKey"], Value::Null);
        assert_eq!(j["isFinished"], false);
        t.set_finished(None, true);
        m.finished(&sockets, &t);
        assert!(m.tasks().is_empty());
        let failed = m.emit_failed(&sockets, "x", TaskText::plain("X"), None, TaskText::plain("It broke"));
        assert!(failed.is_failed && failed.is_finished && m.tasks().is_empty());
    }
}
