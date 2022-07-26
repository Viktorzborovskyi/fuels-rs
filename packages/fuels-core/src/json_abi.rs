use crate::utils::first_four_bytes_of_sha256_hash;
use crate::Token;
use crate::{abi_decoder::ABIDecoder, abi_encoder::ABIEncoder};
use fuels_types::function_selector::build_fn_selector;
use fuels_types::utils::has_array_format;
use fuels_types::{errors::Error, param_types::ParamType, JsonABI, Property};
use hex::FromHex;
use itertools::Itertools;
use serde_json;
use std::str;

pub struct ABIParser {
    fn_selector: Option<Vec<u8>>,
}

impl Default for ABIParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ABIParser {
    pub fn new() -> Self {
        ABIParser { fn_selector: None }
    }

    /// Higher-level layer of the ABI encoding module.
    /// Encode is essentially a wrapper of [`crate::abi_encoder`],
    /// but it is responsible for parsing strings into proper [`Token`]
    /// that can be encoded by the [`crate::abi_encoder`].
    /// Note that `encode` only encodes the parameters for an ABI call,
    /// It won't include the function selector in it. To get the function
    /// selector, use `encode_with_function_selector`.
    ///
    /// # Examples
    /// ```
    /// use fuels_core::json_abi::ABIParser;
    /// let json_abi = r#"
    ///     [
    ///         {
    ///             "type":"contract",
    ///             "inputs":[
    ///                 {
    ///                     "name":"arg",
    ///                     "type":"u32"
    ///                 }
    ///             ],
    ///             "name":"takes_u32_returns_bool",
    ///             "outputs":[
    ///                 {
    ///                     "name":"",
    ///                     "type":"bool"
    ///                 }
    ///             ]
    ///         }
    ///     ]
    ///     "#;
    ///
    ///     let values: Vec<String> = vec!["10".to_string()];
    ///
    ///     let mut abi = ABIParser::new();
    ///
    ///     let function_name = "takes_u32_returns_bool";
    ///     let encoded = abi.encode(json_abi, function_name, &values).unwrap();
    ///     let expected_encode = "000000000000000a";
    ///     assert_eq!(encoded, expected_encode);
    /// ```
    pub fn encode(&mut self, abi: &str, fn_name: &str, values: &[String]) -> Result<String, Error> {
        let parsed_abi: JsonABI = serde_json::from_str(abi)?;

        let entry = parsed_abi.iter().find(|e| e.name == fn_name);

        let entry = entry.expect("No functions found");

        let fn_selector = build_fn_selector(fn_name, &entry.inputs)?;

        // Update the fn_selector field with the hash of the previously encoded function selector
        self.fn_selector = Some(first_four_bytes_of_sha256_hash(&fn_selector).to_vec());

        let params_and_values = entry
            .inputs
            .iter()
            .zip(values)
            .map(|(prop, val)| Ok((ParamType::try_from(prop)?, val.as_str())))
            .collect::<Result<Vec<_>, Error>>()?;

        let tokens = self.parse_tokens(&params_and_values)?;

        Ok(hex::encode(ABIEncoder::encode(&tokens)?))
    }

    /// Similar to `encode`, but includes the function selector in the
    /// final encoded string.
    ///
    /// # Examples
    /// ```
    /// use fuels_core::json_abi::ABIParser;
    /// let json_abi = r#"
    ///     [
    ///         {
    ///             "type":"contract",
    ///             "inputs":[
    ///                 {
    ///                     "name":"arg",
    ///                     "type":"u32"
    ///                 }
    ///             ],
    ///             "name":"takes_u32_returns_bool",
    ///             "outputs":[
    ///                 {
    ///                     "name":"",
    ///                     "type":"bool"
    ///                 }
    ///             ]
    ///         }
    ///     ]
    ///     "#;
    ///
    ///     let values: Vec<String> = vec!["10".to_string()];
    ///
    ///     let mut abi = ABIParser::new();
    ///     let function_name = "takes_u32_returns_bool";
    ///
    ///     let encoded = abi
    ///         .encode_with_function_selector(json_abi, function_name, &values)
    ///         .unwrap();
    ///
    ///     let expected_encode = "000000006355e6ee000000000000000a";
    ///     assert_eq!(encoded, expected_encode);
    /// ```
    pub fn encode_with_function_selector(
        &mut self,
        abi: &str,
        fn_name: &str,
        values: &[String],
    ) -> Result<String, Error> {
        let encoded_params = self.encode(abi, fn_name, values)?;
        let fn_selector = self
            .fn_selector
            .to_owned()
            .expect("Function selector not encoded");

        let encoded_function_selector = hex::encode(fn_selector);

        Ok(format!("{}{}", encoded_function_selector, encoded_params))
    }

    /// Similar to `encode`, but it encodes only an array of strings containing
    /// [<type_1>, <param_1>, <type_2>, <param_2>, <type_n>, <param_n>]
    /// Without having to reference to a JSON specification of the ABI.
    pub fn encode_params(&self, params: &[String]) -> Result<String, Error> {
        let pairs: Vec<_> = params.chunks(2).collect_vec();

        let mut param_type_pairs: Vec<(ParamType, &str)> = vec![];

        for pair in pairs {
            let prop = Property {
                name: "".to_string(),
                type_field: pair[0].clone(),
                components: None,
            };
            let p = ParamType::try_from(&prop)?;

            let t: (ParamType, &str) = (p, &pair[1]);
            param_type_pairs.push(t);
        }

        let tokens = self.parse_tokens(&param_type_pairs)?;

        let encoded = ABIEncoder::encode(&tokens)?;

        Ok(hex::encode(encoded))
    }

    /// Helper function to turn a list of tuples(ParamType, &str) into
    /// a vector of Tokens ready to be encoded.
    /// Essentially a wrapper on `tokenize`.
    pub fn parse_tokens<'a>(&self, params: &'a [(ParamType, &str)]) -> Result<Vec<Token>, Error> {
        params
            .iter()
            .map(|&(ref param, value)| self.tokenize(param, value.to_string()))
            .collect::<Result<_, _>>()
            .map_err(From::from)
    }

    /// Takes a ParamType and a value string and joins them as a single
    /// Token that holds the value within it. This Token is used
    /// in the encoding process.
    pub fn tokenize(&self, param: &ParamType, value: String) -> Result<Token, Error> {
        let trimmed_value = value.trim();
        match &*param {
            ParamType::Unit => Ok(Token::Unit),
            ParamType::U8 => Ok(Token::U8(trimmed_value.parse::<u8>()?)),
            ParamType::U16 => Ok(Token::U16(trimmed_value.parse::<u16>()?)),
            ParamType::U32 => Ok(Token::U32(trimmed_value.parse::<u32>()?)),
            ParamType::U64 => Ok(Token::U64(trimmed_value.parse::<u64>()?)),
            ParamType::Bool => Ok(Token::Bool(trimmed_value.parse::<bool>()?)),
            ParamType::Byte => Ok(Token::Byte(trimmed_value.parse::<u8>()?)),
            ParamType::B256 => {
                const B256_HEX_ENC_LENGTH: usize = 64;
                if trimmed_value.len() != B256_HEX_ENC_LENGTH {
                    return Err(Error::InvalidData(format!(
                        "the hex encoding of the b256 must have {} characters",
                        B256_HEX_ENC_LENGTH
                    )));
                }
                let v = Vec::from_hex(trimmed_value)?;
                let s: [u8; 32] = v.as_slice().try_into().unwrap();
                Ok(Token::B256(s))
            }
            ParamType::Array(t, _) => Ok(self.tokenize_array(trimmed_value, &*t)?),
            ParamType::String(_) => Ok(Token::String(trimmed_value.to_string())),
            ParamType::Struct(struct_params) => {
                Ok(self.tokenize_struct(trimmed_value, struct_params)?)
            }
            ParamType::Enum(variants) => {
                let discriminant = self.get_enum_discriminant_from_string(trimmed_value);
                let value = self.get_enum_value_from_string(trimmed_value);

                let token = self.tokenize(&variants.param_types()[discriminant], value)?;

                Ok(Token::Enum(Box::new((
                    discriminant as u8,
                    token,
                    variants.clone(),
                ))))
            }
            ParamType::Tuple(tuple_params) => Ok(self.tokenize_tuple(trimmed_value, tuple_params)?),
        }
    }

    /// Creates a `Token::Struct` from an array of parameter types and a string of values.
    /// I.e. it takes a string containing values "value_1, value_2, value_3" and an array
    /// of `ParamType` containing the type of each value, in order:
    /// [ParamType::<Type of value_1>, ParamType::<Type of value_2>, ParamType::<Type of value_3>]
    /// And attempts to return a `Token::Struct()` containing the inner types.
    /// It works for nested/recursive structs.
    pub fn tokenize_struct(&self, value: &str, params: &[ParamType]) -> Result<Token, Error> {
        if !value.starts_with('(') || !value.ends_with(')') {
            return Err(Error::InvalidData(
                "struct value string must start and end with round brackets".into(),
            ));
        }

        if value.chars().count() == 2 {
            return Ok(Token::Struct(vec![]));
        }

        //To parse the value string we use a two pointer/index approach.
        //The items are comma separated and if an item is tokenized the last_item
        //index is moved to the current position.
        //The variable nested is incremented and decremented if a bracket is encountered,
        //and appropriate errors are returned if the nested count is not 0.
        //If the struct has an array inside its values the current position will be incremented
        //until both the opening and closing bracket are inside the new item.
        //Characters inside quotes are ignored and they are tokenized as one item.
        //An error is return if there is an odd number of quotes.
        let mut result = vec![];
        let mut nested = 0isize;
        let mut ignore = false;
        let mut last_item = 1;
        let mut params_iter = params.iter();

        for (pos, ch) in value.chars().enumerate() {
            match ch {
                '(' if !ignore => {
                    nested += 1;
                }
                ')' if !ignore => {
                    nested -= 1;

                    match nested.cmp(&0) {
                        std::cmp::Ordering::Less => {
                            return Err(Error::InvalidData(
                                "struct value string has excess closing brackets".into(),
                            ));
                        }
                        std::cmp::Ordering::Equal => {
                            let sub = &value[last_item..pos];

                            let token = self.tokenize(
                                params_iter.next().ok_or_else(|| {
                                    Error::InvalidData(
                                        "struct value contains more elements than the parameter types provided".into(),
                                    )
                                })?,
                                sub.to_string(),
                            )?;
                            result.push(token);
                            last_item = pos + 1;
                        }
                        _ => {}
                    }
                }
                '"' => {
                    ignore = !ignore;
                }
                ',' if nested == 1 && !ignore => {
                    let sub = &value[last_item..pos];
                    // If we've encountered an array within a struct property
                    // keep iterating until we see the end of it "]".
                    if sub.contains('[') && !sub.contains(']') {
                        continue;
                    }

                    let token = self.tokenize(
                        params_iter.next().ok_or_else(|| {
                            Error::InvalidData(
                                "struct value contains more elements than the parameter types provided".into(),
                            )
                        })?,
                        sub.to_string(),
                    )?;
                    result.push(token);
                    last_item = pos + 1;
                }
                _ => (),
            }
        }

        if ignore {
            return Err(Error::InvalidData(
                "struct value string has excess quotes".into(),
            ));
        }

        if nested > 0 {
            return Err(Error::InvalidData(
                "struct value string has excess opening brackets".into(),
            ));
        }

        Ok(Token::Struct(result))
    }

    /// Creates a `Token::Array` from one parameter type and a string of values. I.e. it takes a
    /// string containing values "value_1, value_2, value_3" and a `ParamType` sepecifying the type.
    /// It works for nested/recursive arrays.
    pub fn tokenize_array<'a>(&self, value: &'a str, param: &ParamType) -> Result<Token, Error> {
        if !value.starts_with('[') || !value.ends_with(']') {
            return Err(Error::InvalidData(
                "array value string must start and end with square brackets".into(),
            ));
        }

        if value.chars().count() == 2 {
            return Ok(Token::Array(vec![]));
        }

        //for more details about this algorithm, refer to the tokenize_struct method
        let mut result = vec![];
        let mut nested = 0isize;
        let mut ignore = false;
        let mut last_item = 1;
        for (i, ch) in value.chars().enumerate() {
            match ch {
                '[' if !ignore => {
                    nested += 1;
                }
                ']' if !ignore => {
                    nested -= 1;

                    match nested.cmp(&0) {
                        std::cmp::Ordering::Less => {
                            return Err(Error::InvalidData(
                                "array value string has excess closing brackets".into(),
                            ));
                        }
                        std::cmp::Ordering::Equal => {
                            // Last element of this nest level; proceed to tokenize.
                            let sub = &value[last_item..i];
                            match has_array_format(sub) {
                                true => {
                                    let arr_param = ParamType::Array(
                                        Box::new(param.to_owned()),
                                        self.get_array_length_from_string(sub),
                                    );

                                    result.push(self.tokenize(&arr_param, sub.to_string())?);
                                }
                                false => {
                                    result.push(self.tokenize(param, sub.to_string())?);
                                }
                            }

                            last_item = i + 1;
                        }
                        _ => {}
                    }
                }
                '"' => {
                    ignore = !ignore;
                }
                ',' if nested == 1 && !ignore => {
                    let sub = &value[last_item..i];
                    match has_array_format(sub) {
                        true => {
                            let arr_param = ParamType::Array(
                                Box::new(param.to_owned()),
                                self.get_array_length_from_string(sub),
                            );

                            result.push(self.tokenize(&arr_param, sub.to_string())?);
                        }
                        false => {
                            result.push(self.tokenize(param, sub.to_string())?);
                        }
                    }
                    last_item = i + 1;
                }
                _ => (),
            }
        }

        if ignore {
            return Err(Error::InvalidData(
                "array value string has excess quotes".into(),
            ));
        }

        if nested > 0 {
            return Err(Error::InvalidData(
                "array value string has excess opening brackets".into(),
            ));
        }

        Ok(Token::Array(result))
    }

    /// Creates `Token::Tuple` from an array of parameter types and a string of values.
    /// I.e. it takes a string containing values "value_1, value_2, value_3" and an array
    /// of `ParamType` containing the type of each value, in order:
    /// [ParamType::<Type of value_1>, ParamType::<Type of value_2>, ParamType::<Type of value_3>]
    /// And attempts to return a `Token::Tuple()` containing the inner types.
    /// It works for nested/recursive tuples.
    pub fn tokenize_tuple(&self, value: &str, params: &[ParamType]) -> Result<Token, Error> {
        if !value.starts_with('(') || !value.ends_with(')') {
            return Err(Error::InvalidData(
                "tuple value string must start and end with round brackets".into(),
            ));
        }

        if value.chars().count() == 2 {
            return Ok(Token::Tuple(vec![]));
        }

        //for more details about this algorithm, refer to the tokenize_struct method
        let mut result = vec![];
        let mut nested = 0isize;
        let mut ignore = false;
        let mut last_item = 1;
        let mut params_iter = params.iter();

        for (pos, ch) in value.chars().enumerate() {
            match ch {
                '(' if !ignore => {
                    nested += 1;
                }
                ')' if !ignore => {
                    nested -= 1;

                    match nested.cmp(&0) {
                        std::cmp::Ordering::Less => {
                            return Err(Error::InvalidData(
                                "tuple value string has excess closing brackets".into(),
                            ));
                        }
                        std::cmp::Ordering::Equal => {
                            let sub = &value[last_item..pos];

                            let token = self.tokenize(
                                params_iter.next().ok_or_else(|| {
                                    Error::InvalidData(
                                        "tuple value contains more elements than the parameter types provided".into(),
                                    )
                                })?,
                                sub.to_string(),
                            )?;
                            result.push(token);
                            last_item = pos + 1;
                        }
                        _ => {}
                    }
                }
                '"' => {
                    ignore = !ignore;
                }
                ',' if nested == 1 && !ignore => {
                    let sub = &value[last_item..pos];
                    // If we've encountered an array within a tuple property
                    // keep iterating until we see the end of it "]".
                    if sub.contains('[') && !sub.contains(']') {
                        continue;
                    }

                    let token = self.tokenize(
                        params_iter.next().ok_or_else(|| {
                            Error::InvalidData(
                                "tuple value contains more elements than the parameter types provided".into(),
                            )
                        })?,
                        sub.to_string(),
                    )?;
                    result.push(token);
                    last_item = pos + 1;
                }
                _ => (),
            }
        }

        if ignore {
            return Err(Error::InvalidData(
                "tuple value string has excess quotes".into(),
            ));
        }

        if nested > 0 {
            return Err(Error::InvalidData(
                "tuple value string has excess opening brackets".into(),
            ));
        }

        Ok(Token::Tuple(result))
    }

    /// Higher-level layer of the ABI decoding module.
    /// Decodes a value of a given ABI and a target function's output.
    /// Note that the `value` has to be a byte array, meaning that
    /// the caller must properly cast the "upper" type into a `&[u8]`,
    pub fn decode<'a>(
        &self,
        abi: &str,
        fn_name: &str,
        value: &'a [u8],
    ) -> Result<Vec<Token>, Error> {
        let parsed_abi: JsonABI = serde_json::from_str(abi)?;

        let entry = parsed_abi.iter().find(|e| e.name == fn_name);

        if entry.is_none() {
            return Err(Error::InvalidName(format!(
                "couldn't find function name: {}",
                fn_name
            )));
        }

        let params_result: Result<Vec<_>, _> = entry
            .unwrap()
            .outputs
            .iter()
            .map(ParamType::try_from)
            .collect();

        match params_result {
            Ok(params) => Ok(ABIDecoder::decode(&params, value)?),
            Err(e) => Err(e),
        }
    }

    /// Similar to decode, but it decodes only an array types and the encoded data
    /// without having to reference to a JSON specification of the ABI.
    pub fn decode_params(&self, params: &[ParamType], data: &[u8]) -> Result<Vec<Token>, Error> {
        Ok(ABIDecoder::decode(params, data)?)
    }

    fn get_enum_discriminant_from_string(&self, ele: &str) -> usize {
        let mut chars = ele.chars();
        chars.next(); // Remove "("
        chars.next_back(); // Remove ")"
        let v: Vec<_> = chars.as_str().split(',').collect();
        v[0].parse().unwrap()
    }

    fn get_enum_value_from_string(&self, ele: &str) -> String {
        let mut chars = ele.chars();
        chars.next(); // Remove "("
        chars.next_back(); // Remove ")"
        let v: Vec<_> = chars.as_str().split(',').collect();
        v[1].to_string()
    }

    fn get_array_length_from_string(&self, ele: &str) -> usize {
        let mut chars = ele.chars();
        chars.next();
        chars.next_back();
        chars.as_str().split(',').count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fuels_types::{errors::Error, param_types::ParamType};

    #[test]
    fn simple_encode_and_decode_no_selector() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"arg",
                        "type":"u32"
                    }
                ],
                "name":"takes_u32_returns_bool",
                "outputs":[
                    {
                        "name":"",
                        "type":"bool"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec!["10".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_u32_returns_bool";

        let encoded = abi.encode(json_abi, function_name, &values)?;

        let expected_encode = "000000000000000a";
        assert_eq!(encoded, expected_encode);

        let return_value = [
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, // false
        ];

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let expected_return = vec![Token::Bool(false)];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn simple_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"arg",
                        "type":"u32"
                    }
                ],
                "name":"takes_u32_returns_bool",
                "outputs":[
                    {
                        "name":"",
                        "type":"bool"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec!["10".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_u32_returns_bool";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "000000006355e6ee000000000000000a";
        assert_eq!(encoded, expected_encode);

        let return_value = [
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, // false
        ];

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let expected_return = vec![Token::Bool(false)];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn b256_and_single_byte_encode_and_decode() -> Result<(), Box<dyn std::error::Error>> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"foo",
                        "type":"b256"
                    },
                    {
                        "name":"bar",
                        "type":"byte"
                    }
                ],
                "name":"my_func",
                "outputs":[
                    {
                        "name":"",
                        "type":"b256"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec![
            "d5579c46dfcc7f18207013e65b44e4cb4e2c2298f4ac457ba8f82743f31e930b".to_string(),
            "1".to_string(),
        ];

        let mut abi = ABIParser::new();

        let function_name = "my_func";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "00000000e64019abd5579c46dfcc7f18207013e65b44e4cb4e2c2298f4ac457ba8f82743f31e930b0000000000000001";
        assert_eq!(encoded, expected_encode);

        let return_value =
            hex::decode("a441b15fe9a3cf56661190a0b93b9dec7d04127288cc87250967cf3b52894d11")?;

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let s: [u8; 32] = return_value.as_slice().try_into()?;

        let expected_return = vec![Token::B256(s)];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn array_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"arg",
                        "type":"[u16; 3]"
                    }
                ],
                "name":"takes_array",
                "outputs":[
                    {
                        "name":"",
                        "type":"[u16; 2]"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec!["[1,2,3]".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_array";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "00000000101cbeb5000000000000000100000000000000020000000000000003";
        assert_eq!(encoded, expected_encode);

        let return_value = [
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, // 0
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, // 1
        ];

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let expected_return = vec![Token::Array(vec![Token::U16(0), Token::U16(1)])];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn tokenize_array() -> Result<(), Error> {
        let abi = ABIParser::new();

        let value = "[[1,2],[3],4]";
        let param = ParamType::U16;
        let tokens = abi.tokenize_array(value, &param)?;

        let expected_tokens = Token::Array(vec![
            Token::Array(vec![Token::U16(1), Token::U16(2)]), // First element, a sub-array with 2 elements
            Token::Array(vec![Token::U16(3)]), // Second element, a sub-array with 1 element
            Token::U16(4),                     // Third element
        ]);

        assert_eq!(tokens, expected_tokens);

        let value = "[1,[2],[3],[4,5]]";
        let param = ParamType::U16;
        let tokens = abi.tokenize_array(value, &param)?;

        let expected_tokens = Token::Array(vec![
            Token::U16(1),
            Token::Array(vec![Token::U16(2)]),
            Token::Array(vec![Token::U16(3)]),
            Token::Array(vec![Token::U16(4), Token::U16(5)]),
        ]);

        assert_eq!(tokens, expected_tokens);

        let value = "[1,2,3,4,5]";
        let param = ParamType::U16;
        let tokens = abi.tokenize_array(value, &param)?;

        let expected_tokens = Token::Array(vec![
            Token::U16(1),
            Token::U16(2),
            Token::U16(3),
            Token::U16(4),
            Token::U16(5),
        ]);

        assert_eq!(tokens, expected_tokens);

        let value = "[[1,2,3,[4,5]]]";
        let param = ParamType::U16;
        let tokens = abi.tokenize_array(value, &param)?;

        let expected_tokens = Token::Array(vec![Token::Array(vec![
            Token::U16(1),
            Token::U16(2),
            Token::U16(3),
            Token::Array(vec![Token::U16(4), Token::U16(5)]),
        ])]);

        assert_eq!(tokens, expected_tokens);
        Ok(())
    }

    #[test]
    fn nested_array_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"arg",
                        "type":"[u16; 3]"
                    }
                ],
                "name":"takes_nested_array",
                "outputs":[
                    {
                        "name":"",
                        "type":"[u16; 2]"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec!["[[1,2],[3],[4]]".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_nested_array";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode =
            "00000000e6a030f00000000000000001000000000000000200000000000000030000000000000004";
        assert_eq!(encoded, expected_encode);

        let return_value = [
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, // 0
            0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, // 1
        ];

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let expected_return = vec![Token::Array(vec![Token::U16(0), Token::U16(1)])];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn string_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"foo",
                        "type":"str[23]"
                    }
                ],
                "name":"takes_string",
                "outputs":[
                    {
                        "name":"",
                        "type":"str[2]"
                    }
                ]
            }
        ]
        "#;

        let values: Vec<String> = vec!["This is a full sentence".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_string";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "00000000d56e76515468697320697320612066756c6c2073656e74656e636500";
        assert_eq!(encoded, expected_encode);

        let return_value = [
            0x4f, 0x4b, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, // "OK" encoded in utf8
        ];

        let decoded_return = abi.decode(json_abi, function_name, &return_value)?;

        let expected_return = vec![Token::String("OK".into())];

        assert_eq!(decoded_return, expected_return);
        Ok(())
    }

    #[test]
    fn struct_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"my_struct",
                        "type":"struct MyStruct",
                        "components": [
                            {
                                "name": "foo",
                                "type": "u8"
                            },
                            {
                                "name": "bar",
                                "type": "bool"
                            }
                        ]
                    }
                ],
                "name":"takes_struct",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["(42, true)".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_struct";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "00000000cb0b2f05000000000000002a0000000000000001";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn struct_and_primitive_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"my_struct",
                        "type":"struct MyStruct",
                        "components": [
                            {
                                "name": "foo",
                                "type": "u8"
                            },
                            {
                                "name": "bar",
                                "type": "bool"
                            }
                        ]
                    },
                    {
                        "name":"foo",
                        "type":"u32"
                    }
                ],
                "name":"takes_struct_and_primitive",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["(42, true)".to_string(), "10".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_struct_and_primitive";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "000000005c445838000000000000002a0000000000000001000000000000000a";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn nested_struct_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"top_value",
                        "type":"struct MyNestedStruct",
                        "components": [
                            {
                                "name": "x",
                                "type": "u16"
                            },
                            {
                                "name": "inner",
                                "type": "struct Y",
                                "components": [
                                    {
                                        "name":"a",
                                        "type": "bool"
                                    },
                                    {
                                        "name":"b",
                                        "type": "[u8; 2]"
                                    }
                                ]
                            }
                        ]
                    }
                ],
                "name":"takes_nested_struct",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["(10, (true, [1,2]))".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_nested_struct";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode =
            "00000000b1fbe7e3000000000000000a000000000000000100000000000000010000000000000002";
        assert_eq!(encoded, expected_encode);

        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"top_value",
                        "type":"struct MyNestedStruct",
                        "components": [
                            {
                                "name": "inner",
                                "type": "struct X",
                                "components": [
                                    {
                                        "name":"a",
                                        "type": "bool"
                                    },
                                    {
                                        "name":"b",
                                        "type": "[u8; 2]"
                                    }
                                ]
                            },
                            {
                                "name": "y",
                                "type": "u16"
                            }
                        ]
                    }
                ],
                "name":"takes_nested_struct",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["((true, [1,2]), 10)".to_string()];

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode =
            "00000000e748f310000000000000000100000000000000010000000000000002000000000000000a";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn tuple_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs": [
                  {
                    "name": "input",
                    "type": "(u64, bool)",
                    "components": [
                      {
                        "name": "__tuple_element",
                        "type": "u64",
                        "components": null
                      },
                      {
                        "name": "__tuple_element",
                        "type": "bool",
                        "components": null
                      }
                    ]
                  }
                ],
                "name":"takes_tuple",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["(42, true)".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_tuple";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "000000001cc7bb2c000000000000002a0000000000000001";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn nested_tuple_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
          {
            "type": "function",
            "inputs": [
              {
                "name": "input",
                "type": "((u64, bool), struct Person, enum State)",
                "components": [
                  {
                    "name": "__tuple_element",
                    "type": "(u64, bool)",
                    "components": [
                      {
                        "name": "__tuple_element",
                        "type": "u64",
                        "components": null
                      },
                      {
                        "name": "__tuple_element",
                        "type": "bool",
                        "components": null
                      }
                    ]
                  },
                  {
                    "name": "__tuple_element",
                    "type": "struct Person",
                    "components": [
                      {
                        "name": "name",
                        "type": "str[4]",
                        "components": null
                      }
                    ]
                  },
                  {
                    "name": "__tuple_element",
                    "type": "enum State",
                    "components": [
                      {
                        "name": "A",
                        "type": "()",
                        "components": []
                      },
                      {
                        "name": "B",
                        "type": "()",
                        "components": []
                      },
                      {
                        "name": "C",
                        "type": "()",
                        "components": []
                      }
                    ]
                  }
                ]
              }
            ],
            "name": "takes_nested_tuple",
            "outputs":[]
          }
        ]
        "#;

        let values: Vec<String> = vec!["((42, true), (John), (1, 0))".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_nested_tuple";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        println!("Function: {}", hex::encode(abi.fn_selector.unwrap()));
        let expected_encode =
            "00000000ebb8d011000000000000002a00000000000000014a6f686e000000000000000000000001";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn enum_encode_and_decode() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "type":"contract",
                "inputs":[
                    {
                        "name":"my_enum",
                        "type":"enum MyEnum",
                        "components": [
                            {
                                "name": "x",
                                "type": "u32"
                            },
                            {
                                "name": "y",
                                "type": "bool"
                            }
                        ]
                    }
                ],
                "name":"takes_enum",
                "outputs":[]
            }
        ]
        "#;

        let values: Vec<String> = vec!["(0, 42)".to_string()];

        let mut abi = ABIParser::new();

        let function_name = "takes_enum";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "0000000021b2784f0000000000000000000000000000002a";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn fn_selector_single_primitive() -> Result<(), Error> {
        let p = Property {
            name: "foo".into(),
            type_field: "u64".into(),
            components: None,
        };
        let params = vec![p];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(u64)");
        Ok(())
    }

    #[test]
    fn fn_selector_multiple_primitives() -> Result<(), Error> {
        let p1 = Property {
            name: "foo".into(),
            type_field: "u64".into(),
            components: None,
        };
        let p2 = Property {
            name: "bar".into(),
            type_field: "bool".into(),
            components: None,
        };
        let params = vec![p1, p2];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(u64,bool)");
        Ok(())
    }

    #[test]
    fn fn_selector_custom_type() -> Result<(), Error> {
        let inner_foo = Property {
            name: "foo".into(),
            type_field: "bool".into(),
            components: None,
        };

        let inner_bar = Property {
            name: "bar".into(),
            type_field: "u64".into(),
            components: None,
        };

        let p_struct = Property {
            name: "my_struct".into(),
            type_field: "struct MyStruct".into(),
            components: Some(vec![inner_foo.clone(), inner_bar.clone()]),
        };

        let params = vec![p_struct];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(s(bool,u64))");

        let p_enum = Property {
            name: "my_enum".into(),
            type_field: "enum MyEnum".into(),
            components: Some(vec![inner_foo, inner_bar]),
        };
        let params = vec![p_enum];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(e(bool,u64))");
        Ok(())
    }

    #[test]
    fn fn_selector_nested_struct() -> Result<(), Error> {
        let inner_foo = Property {
            name: "foo".into(),
            type_field: "bool".into(),
            components: None,
        };

        let inner_a = Property {
            name: "a".into(),
            type_field: "u64".into(),
            components: None,
        };

        let inner_b = Property {
            name: "b".into(),
            type_field: "u32".into(),
            components: None,
        };

        let inner_bar = Property {
            name: "bar".into(),
            type_field: "struct InnerStruct".into(),
            components: Some(vec![inner_a, inner_b]),
        };

        let p = Property {
            name: "my_struct".into(),
            type_field: "struct MyStruct".into(),
            components: Some(vec![inner_foo, inner_bar]),
        };

        let params = vec![p];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(s(bool,s(u64,u32)))");
        Ok(())
    }

    #[test]
    fn fn_selector_nested_enum() -> Result<(), Error> {
        let inner_foo = Property {
            name: "foo".into(),
            type_field: "bool".into(),
            components: None,
        };

        let inner_a = Property {
            name: "a".into(),
            type_field: "u64".into(),
            components: None,
        };

        let inner_b = Property {
            name: "b".into(),
            type_field: "u32".into(),
            components: None,
        };

        let inner_bar = Property {
            name: "bar".into(),
            type_field: "enum InnerEnum".into(),
            components: Some(vec![inner_a, inner_b]),
        };

        let p = Property {
            name: "my_enum".into(),
            type_field: "enum MyEnum".into(),
            components: Some(vec![inner_foo, inner_bar]),
        };

        let params = vec![p];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(e(bool,e(u64,u32)))");
        Ok(())
    }

    #[test]
    fn fn_selector_nested_custom_types() -> Result<(), Error> {
        let inner_foo = Property {
            name: "foo".into(),
            type_field: "bool".into(),
            components: None,
        };

        let inner_a = Property {
            name: "a".into(),
            type_field: "u64".into(),
            components: None,
        };

        let inner_b = Property {
            name: "b".into(),
            type_field: "u32".into(),
            components: None,
        };

        let mut inner_custom = Property {
            name: "bar".into(),
            type_field: "enum InnerEnum".into(),
            components: Some(vec![inner_a, inner_b]),
        };

        let p = Property {
            name: "my_struct".into(),
            type_field: "struct MyStruct".into(),
            components: Some(vec![inner_foo.clone(), inner_custom.clone()]),
        };

        let params = vec![p];
        let selector = build_fn_selector("my_func", &params)?;

        assert_eq!(selector, "my_func(s(bool,e(u64,u32)))");

        inner_custom.type_field = "struct InnerStruct".to_string();
        let p = Property {
            name: "my_enum".into(),
            type_field: "enum MyEnum".into(),
            components: Some(vec![inner_foo, inner_custom]),
        };
        let params = vec![p];
        let selector = build_fn_selector("my_func", &params)?;
        assert_eq!(selector, "my_func(e(bool,s(u64,u32)))");
        Ok(())
    }

    #[test]
    fn compiler_generated_abi_test() -> Result<(), Error> {
        let json_abi = r#"
        [
            {
                "inputs": [
                    {
                        "components": null,
                        "name": "value",
                        "type": "u64"
                    }
                ],
                "name": "foo",
                "outputs": [
                    {
                        "components": null,
                        "name": "",
                        "type": "u64"
                    }
                ],
                "type": "function"
            },
            {
                "inputs": [
                    {
                        "components": [
                            {
                                "components": null,
                                "name": "a",
                                "type": "bool"
                            },
                            {
                                "components": null,
                                "name": "b",
                                "type": "u64"
                            }
                        ],
                        "name": "value",
                        "type": "struct TestStruct"
                    }
                ],
                "name": "boo",
                "outputs": [
                    {
                        "components": [
                            {
                                "components": null,
                                "name": "a",
                                "type": "bool"
                            },
                            {
                                "components": null,
                                "name": "b",
                                "type": "u64"
                            }
                        ],
                        "name": "",
                        "type": "struct TestStruct"
                    }
                ],
                "type": "function"
            }
        ]
        "#;

        let s = "(true, 42)".to_string();

        let values: Vec<String> = vec![s];

        let mut abi = ABIParser::new();

        let function_name = "boo";

        let encoded = abi.encode_with_function_selector(json_abi, function_name, &values)?;

        let expected_encode = "00000000e33a11ce0000000000000001000000000000002a";
        assert_eq!(encoded, expected_encode);
        Ok(())
    }

    #[test]
    fn tokenize_uint_types_expected_error() {
        let abi = ABIParser::new();

        // We test only on U8 as it is the same error on all other unsigned int types
        let error_message = abi
            .tokenize(&ParamType::U8, "2,".to_string())
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Parse integer error: invalid digit found in string",
            error_message
        );
    }

    #[test]
    fn tokenize_bool_expected_error() {
        let abi = ABIParser::new();

        let error_message = abi
            .tokenize(&ParamType::Bool, "True".to_string())
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Parse boolean error: provided string was not `true` or `false`",
            error_message
        );
    }

    #[test]
    fn tokenize_b256_invalid_length_expected_error() {
        let abi = ABIParser::new();

        let value = "d57a9c46dfcc7f18207013e65b44e4cb4e2c2298f4ac457ba8f82743f31e90b".to_string();
        let error_message = abi
            .tokenize(&ParamType::B256, value)
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Invalid data: the hex encoding of the b256 must have 64 characters",
            error_message
        );
    }

    #[test]
    fn tokenize_b256_invalid_character_expected_error() {
        let abi = ABIParser::new();

        let value = "Hd57a9c46dfcc7f18207013e65b44e4cb4e2c2298f4ac457ba8f82743f31e90b".to_string();
        let error_message = abi
            .tokenize(&ParamType::B256, value)
            .unwrap_err()
            .to_string();

        assert!(error_message.contains("Parse hex error: Invalid character"));
    }

    #[test]
    fn tokenize_tuple_invalid_start_end_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "(u64, [u64; 3])".to_string(),
            components: Some(vec![
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Tuple(tuple_params) = ParamType::parse_tuple_param(&params)? {
            let error_message = abi
                .tokenize_tuple("0, [0,0,0])", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value string must start and end with round brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_tuple_excess_opening_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "(u64, [u64; 3])".to_string(),
            components: Some(vec![
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Tuple(tuple_params) = ParamType::parse_tuple_param(&params)? {
            let error_message = abi
                .tokenize_tuple("((0, [0,0,0])", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value string has excess opening brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_tuple_excess_closing_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "(u64, [u64; 3])".to_string(),
            components: Some(vec![
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Tuple(tuple_params) = ParamType::parse_tuple_param(&params)? {
            let error_message = abi
                .tokenize_tuple("(0, [0,0,0]))", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value string has excess closing brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_tuple_excess_quotes_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "(u64, [u64; 3])".to_string(),
            components: Some(vec![
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Tuple(tuple_params) = ParamType::parse_tuple_param(&params)? {
            let error_message = abi
                .tokenize_tuple("(0, \"[0,0,0])", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value string has excess quotes",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_tuple_excess_value_elements_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "(u64, [u64; 3])".to_string(),
            components: Some(vec![
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "__tuple_element".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Tuple(tuple_params) = ParamType::parse_tuple_param(&params)? {
            let error_message = abi
                .tokenize_tuple("(0, [0,0,0], 0, 0)", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value contains more elements than the parameter types provided",
                error_message
            );

            let error_message = abi
                .tokenize_tuple("(0, [0,0,0], 0)", &tuple_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: tuple value contains more elements than the parameter types provided",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_array_invalid_start_end_bracket_expected_error() {
        let param = ParamType::U16;
        let abi = ABIParser::new();

        let error_message = abi
            .tokenize_array("1,2],[3],4]", &param)
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Invalid data: array value string must start and end with square brackets",
            error_message
        );
    }

    #[test]
    fn tokenize_array_excess_opening_bracket_expected_error() {
        let param = ParamType::U16;
        let abi = ABIParser::new();

        let error_message = abi
            .tokenize_array("[[[1,2],[3],4]", &param)
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Invalid data: array value string has excess opening brackets",
            error_message
        );
    }

    #[test]
    fn tokenize_array_excess_closing_bracket_expected_error() {
        let param = ParamType::U16;
        let abi = ABIParser::new();

        let error_message = abi
            .tokenize_array("[[1,2],[3],4]]", &param)
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Invalid data: array value string has excess closing brackets",
            error_message
        );
    }

    #[test]
    fn tokenize_array_excess_quotes_expected_error() {
        let param = ParamType::U16;
        let abi = ABIParser::new();

        let error_message = abi
            .tokenize_array("[[1,\"2],[3],4]]", &param)
            .unwrap_err()
            .to_string();

        assert_eq!(
            "Invalid data: array value string has excess quotes",
            error_message
        );
    }

    #[test]
    fn tokenize_struct_invalid_start_end_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "struct MyStruct".to_string(),
            components: Some(vec![
                Property {
                    name: "num".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "arr".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Struct(struct_params) = ParamType::parse_custom_type_param(&params)? {
            let error_message = abi
                .tokenize_struct("0, [0,0,0])", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value string must start and end with round brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_struct_excess_opening_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "struct MyStruct".to_string(),
            components: Some(vec![
                Property {
                    name: "num".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "arr".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Struct(struct_params) = ParamType::parse_custom_type_param(&params)? {
            let error_message = abi
                .tokenize_struct("((0, [0,0,0])", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value string has excess opening brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_struct_excess_closing_bracket_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "struct MyStruct".to_string(),
            components: Some(vec![
                Property {
                    name: "num".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "arr".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Struct(struct_params) = ParamType::parse_custom_type_param(&params)? {
            let error_message = abi
                .tokenize_struct("(0, [0,0,0]))", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value string has excess closing brackets",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_struct_excess_quotes_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "struct MyStruct".to_string(),
            components: Some(vec![
                Property {
                    name: "num".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "arr".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Struct(struct_params) = ParamType::parse_custom_type_param(&params)? {
            let error_message = abi
                .tokenize_struct("(0, \"[0,0,0])", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value string has excess quotes",
                error_message
            );
        }
        Ok(())
    }

    #[test]
    fn tokenize_struct_excess_value_elements_expected_error() -> Result<(), Error> {
        let abi = ABIParser::new();
        let params = Property {
            name: "input".to_string(),
            type_field: "struct MyStruct".to_string(),
            components: Some(vec![
                Property {
                    name: "num".to_string(),
                    type_field: "u64".to_string(),
                    components: None,
                },
                Property {
                    name: "arr".to_string(),
                    type_field: "[u64; 3]".to_string(),
                    components: None,
                },
            ]),
        };

        if let ParamType::Struct(struct_params) = ParamType::parse_custom_type_param(&params)? {
            let error_message = abi
                .tokenize_struct("(0, [0,0,0], 0, 0)", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value contains more elements than the parameter types provided",
                error_message
            );

            let error_message = abi
                .tokenize_struct("(0, [0,0,0], 0)", &struct_params)
                .unwrap_err()
                .to_string();

            assert_eq!(
                "Invalid data: struct value contains more elements than the parameter types provided",
                error_message
            );
        }
        Ok(())
    }
}
