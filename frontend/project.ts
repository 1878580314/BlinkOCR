import project from "../src-tauri/resources/project.json" with { type: "json" };

export const author = project.author;
export const repository = project.repository;
export const githubUrl = `https://github.com/${repository}`;
export const authorUrl = `https://github.com/${repository.split("/")[0]}`;
