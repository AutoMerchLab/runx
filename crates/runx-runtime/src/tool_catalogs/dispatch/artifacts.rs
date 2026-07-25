use runx_contracts::JsonValue;
use runx_parser::SkillArtifactContract;

use crate::adapter::InvocationOutput;
use crate::execution::output_projection::data_envelope;

pub(super) fn apply(output: &mut InvocationOutput, artifacts: Option<&SkillArtifactContract>) {
    let Some(artifacts) = artifacts else {
        return;
    };
    let JsonValue::Object(object) = &mut output.value else {
        return;
    };

    if let Some(wrap_as) = artifacts.wrap_as.as_deref()
        && !object.contains_key(wrap_as)
    {
        object.insert(
            wrap_as.to_owned(),
            data_envelope(JsonValue::Object(object.clone())),
        );
    }
    if let Some(named_emits) = &artifacts.named_emits {
        for name in named_emits.keys() {
            let Some(value) = object.get(name).cloned() else {
                continue;
            };
            object.insert(name.clone(), data_envelope(value));
        }
    }
}
