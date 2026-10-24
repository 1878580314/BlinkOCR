//! Task availability and prompt routing share the same data with the frontend.
use serde::Deserialize;
use std::{collections::HashMap, sync::LazyLock};

#[derive(Debug, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub prompt: String,
}
#[derive(Debug, Deserialize)]
pub struct Profile {
    pub modes: Vec<String>,
    pub prompts: HashMap<String, String>,
    pub api_tasks: HashMap<String, String>,
    pub image_min_tokens: u32,
    pub max_image_dimension: u32,
}
#[derive(Debug, Deserialize)]
pub struct Profiles {
    pub tasks: Vec<Task>,
    pub models: HashMap<String, Profile>,
}
pub static PROFILES: LazyLock<Profiles> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../resources/profiles.json")).expect("valid model tasks")
});
pub fn profile(id: &str) -> &'static Profile {
    PROFILES.models.get(id).unwrap_or(&PROFILES.models["other"])
}
pub fn task_for_prompt(prompt: &str) -> Option<&'static str> {
    PROFILES
        .tasks
        .iter()
        .find(|task| task.prompt == prompt)
        .map(|task| task.id.as_str())
}
pub fn selector(task: &str) -> &'static str {
    &PROFILES
        .tasks
        .iter()
        .find(|entry| entry.id == task)
        .expect("registered task")
        .prompt
}
pub fn default_prompt(model: &str) -> &'static str {
    selector(&profile(model).modes[0])
}
pub fn selected_task(model: &str, prompt: &str) -> &'static str {
    let profile = profile(model);
    task_for_prompt(prompt)
        .filter(|task| profile.modes.iter().any(|mode| mode == task))
        .unwrap_or(&profile.modes[0])
}
pub fn effective_prompt<'a>(model: &str, prompt: &'a str) -> &'a str {
    if model == "other" && task_for_prompt(prompt).is_none() {
        return prompt;
    }
    let task = selected_task(model, prompt);
    profile(model)
        .prompts
        .get(task)
        .map(String::as_str)
        .unwrap_or_else(|| selector(task))
}
pub fn api_task(model: &str, prompt: &str) -> &'static str {
    let task = selected_task(model, prompt);
    profile(model)
        .api_tasks
        .get(task)
        .map(String::as_str)
        .unwrap_or(task)
}
