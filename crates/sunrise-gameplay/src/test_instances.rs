//! The game test instance registry's entries: `GameTestInstance`'s codec.
use sunrise_client::text::Text;
use sunrise_world::worldgen::structure::Rotation;

/// The `GameTestInstance` a registry entry describes: `TestData` and what
/// `describe()` prints.
#[derive(Clone)]
pub struct TestDef {
    pub structure: String,
    pub rotation: Rotation,
    pub padding: i32,
    pub required: bool,
    pub sky_access: bool,
    /// `BlockBasedTestInstance` (else a `FunctionGameTestInstance`).
    pub block_based: bool,
    /// The function a function test runs.
    pub function: Option<String>,
    pub callback: Option<std::sync::Arc<dyn mod_api::GameTestFunction>>,
    /// The test environment's id (`batch()`).
    pub environment: String,
    pub max_ticks: i32,
    pub setup_ticks: i32,
    /// `TestData.maxAttempts` and `requiredSuccesses`.
    pub max_attempts: i32,
    pub required_successes: i32,
    /// The dimension the test runs in (`TestData.dimension`).
    pub dimension: String,
    pub description: Text,
}

/// `GameTestInstance`'s codec, as the game reads a `test_instance` file.
pub fn parse_test_def(json: &serde_json::Value) -> Option<TestDef> {
    let string = |key: &str| json.get(key).and_then(|v| v.as_str());
    let structure = string("structure")?.to_owned();
    let environment = string("environment")?.to_owned();
    let dimension = string("dimension").unwrap_or("minecraft:overworld");
    let function = string("function");
    // `ExtraCodecs.POSITIVE_INT`, 1 when absent.
    let positive = |key: &str| {
        json.get(key)
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .clamp(1, i64::from(i32::MAX)) as i32
    };
    // GameTestInstance.describe: the type, the function (function tests
    // only), then the structure, batch and dimension.
    let block_based = string("type") == Some("minecraft:block_based");
    let mut description = Text::translate(
        "test_instance.description.type",
        &[Text::translate(
            if block_based {
                "test_instance.type.block_based"
            } else {
                "test_instance.type.function"
            },
            &[],
        )
        .with_color(0x5555FF)],
    )
    .append(Text::literal("\n"));
    if !block_based && let Some(function) = function {
        description = description.append(row("test_instance.description.function", function));
    }
    description = description
        .append(row("test_instance.description.structure", &structure))
        .append(row("test_instance.description.batch", &environment))
        .append(row("test_instance.description.dimension", dimension));
    Some(TestDef {
        structure,
        rotation: string("rotation")
            .and_then(Rotation::from_name)
            .unwrap_or(Rotation::None),
        padding: json
            .get("padding")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .clamp(0, 128) as i32,
        required: json
            .get("required")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        sky_access: json
            .get("sky_access")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        block_based,
        function: function.map(|id| {
            if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            }
        }),
        callback: None,
        environment: environment.clone(),
        dimension: dimension.to_owned(),
        max_ticks: json
            .get("max_ticks")
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .clamp(1, i64::from(i32::MAX)) as i32,
        setup_ticks: json
            .get("setup_ticks")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .clamp(0, i64::from(i32::MAX)) as i32,
        max_attempts: positive("max_attempts"),
        required_successes: positive("required_successes"),
        description,
    })
}
/// `GameTestInstance.descriptionRow`: the row's value in blue, then a line break.
fn row(key: &str, value: &str) -> Text {
    Text::translate(key, &[Text::literal(value).with_color(0x5555FF)]).append(Text::literal("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_print_the_value_in_blue_then_a_line_break() {
        let text = row("test_instance.description.structure", "minecraft:empty");
        assert!(text.plain().ends_with('\n'));
    }
}
