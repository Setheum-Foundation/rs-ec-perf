

use derive_more::{Add, AddAssign, BitXor, BitXorAssign, Sub, SubAssign};


/// Additive via XOR form of f2e16
#[derive(Clone, Copy, Debug, Default, BitXor, BitXorAssign, PartialEq, Eq)] // PartialOrd,Ord
pub struct Additive(pub Elt);

impl Additive {
    #[inline(always)]
	pub fn to_wide(self) -> Wide {
		self.0 as Wide
	}
    #[inline(always)]
	pub fn from_wide(x: Wide) -> Additive {
		Additive(x as Elt)
	}

	pub const ZERO: Additive = Additive(0);
}

#[cfg(table_bootstrap_complete)]
impl Additive {
	/// Return multiplier prepared form
    #[inline(always)]
	pub fn to_multiplier(self) -> Multiplier {
		Multiplier(LOG_TABLE[self.0 as usize])
	}

	/// Return a*EXP_TABLE[b] over GF(2^r)
    #[inline(always)]
	pub fn mul(self, other: Multiplier) -> Additive {
		if self == Self::ZERO {
			return Self::ZERO;
		}
		let log = (LOG_TABLE[self.0 as usize] as Wide) + other.0 as Wide;
		let offset = (log & ONEMASK as Wide) + (log >> FIELD_BITS);
		Additive(EXP_TABLE[offset as usize])
	}

	/// Multiply field elements by a single multiplier, using SIMD if available
    #[inline(always)]
	pub fn mul_assign_slice(selfy: &mut [Self], other: Multiplier) {
		// TODO: SIMD
		for s in selfy {
			*s = s.mul(other);
		}
	}
}


/// Multiplicaiton friendly LOG form of f2e16
#[derive(Clone, Copy, Debug, Add, AddAssign, Sub, SubAssign, PartialEq, Eq)] // Default, PartialOrd,Ord
pub struct Multiplier(pub Elt);

impl Multiplier {
    #[inline(always)]
	pub fn to_wide(self) -> Wide {
		self.0 as Wide
	}
    #[inline(always)]
	pub fn from_wide(x: Wide) -> Multiplier {
		Multiplier(x as Elt)
	}
}


/// Fast Walsh–Hadamard transform over modulo ONEMASK
pub fn walsh(data: &mut [Multiplier], size: usize) {
	let mut depart_no = 1_usize;
	while depart_no < size {
		let mut j = 0;
		let depart_no_next = depart_no << 1;
		while j < size {
			for i in j..(depart_no + j) {
				// We deal with data in log form here, but field form looks like:
				//			 data[i] := data[i] / data[i+depart_no]
				// data[i+depart_no] := data[i] * data[i+depart_no]
				let mask = ONEMASK as Wide;
				let tmp2: Wide = data[i].to_wide() + mask - data[i + depart_no].to_wide();
				let tmp1: Wide = data[i].to_wide() + data[i + depart_no].to_wide();
				data[i] = Multiplier(((tmp1 & mask) + (tmp1 >> FIELD_BITS)) as Elt);
				data[i + depart_no] = Multiplier(((tmp2 & mask) + (tmp2 >> FIELD_BITS)) as Elt);
			}
			j += depart_no_next;
		}
		depart_no = depart_no_next;
	}
}


/*
#[test]
fn cantor_basis() {
    /*
    for b in &BASE {
        let b = Additive(*b);
        let square = b.mul(b.to_multiplier());
        assert!(BASE.contains(& (square ^ b).0 ));
    }
    */
    for w in BASE.windows(2) {
        let b = Additive(w[1]);
        let square = b.mul(b.to_multiplier());
        let a = w[0];
        let eq = if a == (square ^ b).0 { "==" } else { "!=" };
        println!("{:#b} {} {:#b}\n", a, eq, (square ^ b).0);
        // assert_eq!(a, (square ^ b).0);
    }
}
*/