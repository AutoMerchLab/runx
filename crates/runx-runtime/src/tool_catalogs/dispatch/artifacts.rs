use runx_contracts::JsonValue;
use runx_parser::SkillArtifactContract;

use crate::RuntimeError;
use crate::adapter::SkillOutput;
use crate::execution::output_projection::data_envelope;

pub(super) fn apply(
    output: &mut SkillOutput,
    artifacts: Option<&SkillArtifactContract>,
) -> Result<(), RuntimeError> {
    let Some(artifacts) = artifacts else {
        return Ok(());
    };
    let Ok(JsonValue::Object(mut object)) = serde_json::from_str::<JsonValue>(&output.stdout)
    else {
        return Ok(());
    };

    let mut changed = false;
    if let Some(wrap_as) = artifacts.wrap_as.as_deref()
        && !object.contains_key(wrap_as)
    {
        object.insert(
            wrap_as.to_owned(),
            data_envelope(JsonValue::Object(object.clone())),
        );
        changed = true;
    }
    if let Some(named_emits) = &artifacts.named_emits {
        for name in named_emits.keys() {
            let Some(value) = object.get(name).cloned() else {
                continue;
            };
            object.insert(name.clone(), data_envelope(value));
            changed = true;
        }
    }
    if changed {
        output.stdout = serde_json::to_string(&JsonValue::Object(object))
            .map_err(|source| RuntimeError::json("serializing tool artifact wrappers", source))?;
    }
    Ok(())
}
