#![allow(dead_code)]

// Generated from Lean 4 module: SemanticFixture

pub fn allConsecutive(x_1: u64, x_2: &[u64]) -> Result<bool, crate::ComputeError> {
    Ok(match x_2 {
        [] => { let _x_45 = true; _x_45 },
        [head_28, tail_29 @ ..] => { let head_28 = head_28.clone(); { let _x_50 = (x_1 == head_28); match _x_50 {
        false => _x_50,
        true => { let _x_54 = 1; { let _x_55 = ((x_1) as u64).checked_add(_x_54).ok_or(crate::ComputeError::AddOverflow)?; { let _x_56 = allConsecutive(_x_55, &(tail_29))?; _x_56 } } },
    } } },
    })
}

