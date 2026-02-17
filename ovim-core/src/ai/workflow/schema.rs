use anyhow::{anyhow, Context, Result};
use serde_json::Value;

pub fn validate_json_output(instance: &Value, schema: &Value) -> Result<()> {
    let validator = jsonschema::validator_for(schema).context("invalid JSON schema")?;
    if let Err(err) = validator.validate(instance) {
        return Err(anyhow!(
            "schema validation error at '{}': {}",
            err.instance_path,
            err
        ));
    }
    Ok(())
}
