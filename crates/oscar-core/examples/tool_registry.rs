//! Credential-free demonstration of host registration and permission checks.
use oscar_core::tools::*;
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

struct Echo;
impl Tool for Echo {
    fn execute(&self, input: ValidatedInput, context: ToolContext) -> ToolFuture<'_> {
        Box::pin(async move {
            if context.cancellation.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let Some(InputValue::Text(text)) = input.get("text") else {
                return Err(ToolError::Failed);
            };
            if text.len() > context.max_output_bytes {
                return Err(ToolError::OutputLimit);
            }
            Ok(text.clone())
        })
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut registry = ToolRegistry::default();
    registry.register(
        ToolDefinition {
            name: "echo".into(),
            description: "Return bounded text without side effects".into(),
            effect: Effect::ReadOnly,
            input: InputSchema {
                fields: BTreeMap::from([(
                    "text".into(),
                    Field {
                        required: true,
                        value_type: InputType::Text { max_bytes: 256 },
                    },
                )]),
            },
        },
        Arc::new(Echo),
    )?;
    let policy = ToolPolicy {
        allowed_tools: ["echo".into()].into(),
        allow_side_effects: false,
    };
    let mut session = registry.session(
        policy,
        ToolLimits::default(),
        None,
        CancellationToken::new(),
    )?;
    let result = session.execute_json(r#"{"id":"demo-1","name":"echo","arguments":{"text":"Registered tools require explicit host permission."}}"#).await?;
    println!("{result}");
    println!("{}", serde_json::to_string_pretty(session.events())?);
    Ok(())
}
