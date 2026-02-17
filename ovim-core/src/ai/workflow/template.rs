use anyhow::{Context, Result};
use minijinja::{Environment, UndefinedBehavior};
use serde::Serialize;

pub fn validate_template(source: &str) -> Result<()> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_template("workflow_step", source)
        .context("template parse error")?;
    Ok(())
}

pub fn render_template<T: Serialize>(source: &str, ctx: &T) -> Result<String> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    let rendered = env
        .render_str(source, ctx)
        .context("template render error")?;
    Ok(rendered)
}
