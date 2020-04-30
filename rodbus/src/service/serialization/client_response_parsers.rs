use crate::error::details::ADUParseError;
use crate::error::*;
use crate::service::traits::{ParseRequest, ParseResponse};
use crate::types::{coil_from_u16, AddressRange, Indexed, WriteMultiple};
use crate::util::cursor::ReadCursor;

impl ParseResponse<Indexed<u16>> for Indexed<u16> {
    fn parse_response(cursor: &mut ReadCursor, request: &Indexed<u16>) -> Result<Self, Error> {
        let response = Indexed::new(cursor.read_u16_be()?, cursor.read_u16_be()?);

        if request != &response {
            return Err(details::ADUParseError::ReplyEchoMismatch.into());
        }

        Ok(response)
    }
}

impl ParseResponse<Indexed<bool>> for Indexed<bool> {
    fn parse_response(cursor: &mut ReadCursor, request: &Indexed<bool>) -> Result<Self, Error> {
        let response: Indexed<bool> =
            Indexed::new(cursor.read_u16_be()?, coil_from_u16(cursor.read_u16_be()?)?);

        if &response != request {
            return Err(details::ADUParseError::ReplyEchoMismatch.into());
        }

        Ok(response)
    }
}

impl ParseResponse<AddressRange> for Vec<Indexed<bool>> {
    fn parse_response(cursor: &mut ReadCursor, request: &AddressRange) -> Result<Self, Error> {
        let byte_count = cursor.read_u8()? as usize;

        // how many bytes should we have?
        let expected_byte_count = crate::util::bits::num_bytes_for_bits(request.count);

        if byte_count != expected_byte_count {
            return Err(details::ADUParseError::RequestByteCountMismatch(
                expected_byte_count,
                byte_count,
            )
            .into());
        }

        if byte_count != cursor.len() {
            return Err(details::ADUParseError::InsufficientBytesForByteCount(
                byte_count,
                cursor.len(),
            )
            .into());
        }

        let bytes = cursor.read_bytes(byte_count)?;

        let mut values = Vec::<Indexed<bool>>::with_capacity(request.count as usize);

        let mut count = 0;

        for byte in bytes {
            for i in 0..8 {
                // return early if we hit the count before the end of the byte
                if count == request.count {
                    return Ok(values);
                }

                // low order bits first
                let value = (byte & (1u8 << i)) != 0;
                values.push(Indexed::new(count + request.start, value));
                count += 1;
            }
        }

        Ok(values)
    }
}

impl ParseResponse<AddressRange> for Vec<Indexed<u16>> {
    fn parse_response(cursor: &mut ReadCursor, request: &AddressRange) -> Result<Self, Error> {
        let byte_count = cursor.read_u8()? as usize;

        // how many bytes should we have?
        let expected_byte_count = 2 * (request.count as usize);

        if byte_count != expected_byte_count {
            return Err(details::ADUParseError::RequestByteCountMismatch(
                expected_byte_count,
                byte_count,
            )
            .into());
        }

        if expected_byte_count != cursor.len() {
            return Err(details::ADUParseError::InsufficientBytesForByteCount(
                byte_count,
                cursor.len(),
            )
            .into());
        }

        let mut values = Vec::<Indexed<u16>>::with_capacity(request.count as usize);

        let mut index = request.start;

        while !cursor.is_empty() {
            values.push(Indexed::new(index, cursor.read_u16_be()?));
            index += 1;
        }

        Ok(values)
    }
}

impl ParseResponse<WriteMultiple<bool>> for AddressRange {
    fn parse_response(
        cursor: &mut ReadCursor,
        request: &WriteMultiple<bool>,
    ) -> Result<Self, Error> {
        let range = request.to_address_range()?;
        let parsed = AddressRange::parse(cursor)?;
        if range != parsed {
            return Err(ADUParseError::ReplyEchoMismatch.into());
        }
        Ok(parsed)
    }
}

impl ParseResponse<WriteMultiple<u16>> for AddressRange {
    fn parse_response(
        cursor: &mut ReadCursor,
        request: &WriteMultiple<u16>,
    ) -> Result<Self, Error> {
        let range = request.to_address_range()?;
        let parsed = AddressRange::parse(cursor)?;
        if range != parsed {
            return Err(ADUParseError::ReplyEchoMismatch.into());
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vec_of_bool() {
        let input = [0x01, 0b00000101]; // 0b00000101
        let mut cursor = ReadCursor::new(&input);

        let result = Vec::<Indexed<bool>>::parse_response(
            &mut cursor,
            &AddressRange::try_from(0, 3).unwrap(),
        )
        .unwrap();
        let expected = vec![
            Indexed::new(0, true),
            Indexed::new(1, false),
            Indexed::new(2, true),
        ];

        assert_eq!(result, expected);
    }
}
