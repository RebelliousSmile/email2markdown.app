pub const COMMAND_CLSID_U128: u128 = 0xa18325b7_1289_4856_a8bd_69f6d633da13;
pub const COMMAND_CLSID_BRACED: &str = "{A18325B7-1289-4856-A8BD-69F6D633DA13}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braced_and_u128_encode_the_same_guid() {
        let hex = COMMAND_CLSID_BRACED
            .trim_start_matches('{')
            .trim_end_matches('}')
            .replace('-', "");
        let parsed = u128::from_str_radix(&hex, 16).expect("valid hex GUID");
        assert_eq!(parsed, COMMAND_CLSID_U128);
    }
}
