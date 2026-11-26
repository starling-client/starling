pub use world::rule_test::{
    PosRuleTest, PosRuleTestContext, PosRuleTestDecodeContext, PosRuleTestRef, PosRuleTestType,
    RuleTest, RuleTestContext, RuleTestDecodeContext, RuleTestRef, RuleTestType,
};
mod __private {
    /// The registered rule test and position rule test types, and the handles
    /// the game's adapter builds and reads back.
    pub use crate::world::rule_test::{
        pos_rule_test_payload, pos_rule_test_ref, pos_rule_test_type_declarations,
        rule_test_payload, rule_test_ref, rule_test_type_declarations,
    };
}
