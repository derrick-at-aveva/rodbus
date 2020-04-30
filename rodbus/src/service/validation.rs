pub(crate) mod range {
    use crate::error::details::InvalidRequest;
    use crate::types::AddressRange;

    fn check_validity(range: AddressRange, max_count: u16) -> Result<(), InvalidRequest> {
        if range.count > max_count {
            return Err(InvalidRequest::CountTooBigForType(range.count, max_count));
        }
        Ok(())
    }

    pub fn check_validity_for_read_bits(range: AddressRange) -> Result<(), InvalidRequest> {
        check_validity(range, crate::constants::limits::MAX_READ_COILS_COUNT)
    }

    pub fn check_validity_for_read_registers(range: AddressRange) -> Result<(), InvalidRequest> {
        check_validity(range, crate::constants::limits::MAX_READ_REGISTERS_COUNT)
    }

    pub fn check_validity_for_write_multiple_coils(
        range: AddressRange,
    ) -> Result<(), InvalidRequest> {
        check_validity(range, crate::constants::limits::MAX_WRITE_COILS_COUNT)
    }

    pub fn check_validity_for_write_multiple_registers(
        range: AddressRange,
    ) -> Result<(), InvalidRequest> {
        check_validity(range, crate::constants::limits::MAX_WRITE_REGISTERS_COUNT)
    }

    #[cfg(test)]
    mod tests {
        use crate::error::details::InvalidRequest;

        use super::*;

        #[test]
        fn address_range_validates_correctly_for_bits() {
            assert_eq!(
                check_validity_for_read_bits(
                    AddressRange::try_from(0, crate::constants::limits::MAX_READ_COILS_COUNT)
                        .unwrap()
                ),
                Ok(())
            );
            let err = Err(InvalidRequest::CountTooBigForType(
                crate::constants::limits::MAX_READ_COILS_COUNT + 1,
                crate::constants::limits::MAX_READ_COILS_COUNT,
            ));
            assert_eq!(
                check_validity_for_read_bits(
                    AddressRange::try_from(0, crate::constants::limits::MAX_READ_COILS_COUNT + 1)
                        .unwrap()
                ),
                err
            );
        }

        #[test]
        fn address_range_validates_correctly_for_registers() {
            assert_eq!(
                check_validity_for_read_registers(
                    AddressRange::try_from(0, crate::constants::limits::MAX_READ_REGISTERS_COUNT)
                        .unwrap()
                ),
                Ok(())
            );
            let err = Err(InvalidRequest::CountTooBigForType(
                crate::constants::limits::MAX_READ_REGISTERS_COUNT + 1,
                crate::constants::limits::MAX_READ_REGISTERS_COUNT,
            ));
            assert_eq!(
                check_validity_for_read_registers(
                    AddressRange::try_from(
                        0,
                        crate::constants::limits::MAX_READ_REGISTERS_COUNT + 1
                    )
                    .unwrap()
                ),
                err
            );
        }
    }
}
