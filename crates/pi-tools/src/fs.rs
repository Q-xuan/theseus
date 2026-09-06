use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::{pin_workspace, resolve_path, ToolContext, ToolOutput};

const OUTPUT_CAP: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditArgs {
    path: String,
    #[serde(alias = "oldText")]
    old_string: String,
    #[serde(alias = "newText")]
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

pub(crate) fn read_file(arguments: &str, ctx: &ToolContext) -> ToolOutput {
    let args: ReadArgs = match serde_json::from_str(arguments) {
        Ok(a) => a,
        Err(e) => return ToolOutput::error(format!("invalid read arguments: {e}")),
    };
    if let Some(0) = args.offset {
        return ToolOutput::error("offset is 1-based");
    }
    let path = match resolve_path(&ctx.cwd, &args.path) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    let mut text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolOutput::error(format!("{}: {e}", path.display())),
    };
    let mut truncated = false;
    if text.len() > OUTPUT_CAP {
        text.truncate(OUTPUT_CAP);
        truncated = true;
    }
    let lines: Vec<&str> = text.lines().collect();
    let start = args
        .offset
        .map(|n| n.saturating_sub(1) as usize)
        .unwrap_or(0);
    if start >= lines.len() {
        return ToolOutput::ok("(empty)");
    }
    let end = args
        .limit
        .map(|n| start.saturating_add(n as usize).min(lines.len()))
        .unwrap_or(lines.len());
    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let n = start + i + 1;
        out.push_str(&format!("{n:6}|{line}\n"));
    }
    if truncated {
        out.push_str("…(truncated)\n");
    }
    if out.is_empty() {
        out.push_str("(empty)");
    }
    ToolOutput::ok(out)
}

pub(crate) fn write_file(arguments: &str, ctx: &ToolContext) -> ToolOutput {
    let args: WriteArgs = match serde_json::from_str(arguments) {
        Ok(a) => a,
        Err(e) => return ToolOutput::error(format!("invalid write arguments: {e}")),
    };
    let path = match resolve_path(&ctx.cwd, &args.path) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return ToolOutput::error(format!("mkdir {}: {e}", parent.display()));
        }
    }
    match fs::write(&path, args.content.as_bytes()) {
        Ok(()) => ToolOutput::ok(format!(
            "wrote {} bytes to {}",
            args.content.len(),
            display_rel(&ctx.cwd, &path)
        )),
        Err(e) => ToolOutput::error(format!("{}: {e}", path.display())),
    }
}

pub(crate) fn edit_file(arguments: &str, ctx: &ToolContext) -> ToolOutput {
    let args: EditArgs = match serde_json::from_str(arguments) {
        Ok(a) => a,
        Err(e) => return ToolOutput::error(format!("invalid edit arguments: {e}")),
    };
    if args.old_string.is_empty() {
        return ToolOutput::error("oldString is empty");
    }
    let path = match resolve_path(&ctx.cwd, &args.path) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    let content = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolOutput::error(format!("{}: {e}", path.display())),
    };
    let count = content.matches(&args.old_string).count();
    if count == 0 {
        return ToolOutput::error("oldString not found");
    }
    if !args.replace_all && count > 1 {
        return ToolOutput::error(format!(
            "oldString appears {count} times; pass replaceAll=true to replace all"
        ));
    }
    let next = if args.replace_all {
        content.replace(&args.old_string, &args.new_string)
    } else {
        content.replacen(&args.old_string, &args.new_string, 1)
    };
    match fs::write(&path, next.as_bytes()) {
        Ok(()) => ToolOutput::ok(format!(
            "replaced {count} occurrence(s) in {}",
            display_rel(&ctx.cwd, &path)
        )),
        Err(e) => ToolOutput::error(format!("{}: {e}", path.display())),
    }
}

fn display_rel(cwd: &Path, path: &Path) -> String {
    let root = pin_workspace(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    path.strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
