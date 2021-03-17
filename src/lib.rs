
extern crate derive_more;
extern crate static_init;


pub static SMALL_RNG_SEED: [u8; 32] = [
	0, 6, 0xFA, 0, 0x37, 3, 19, 89, 32, 032, 0x37, 0x77, 77, 0b11, 112, 52, 12, 40, 82, 34, 0, 0, 0, 1, 4, 4, 1, 4, 99,
	127, 121, 107,
];

pub mod errors;
pub use errors::*;

mod wrapped_shard;

use rand::prelude::*;
use rand::seq::index::IndexVec;

pub use wrapped_shard::*;

#[cfg(feature = "status-quo")]
pub mod status_quo;

pub mod f256;
pub mod f2e16;

#[test]
fn agreement_f2e16_with_f256() {
    for i in 1..=255 {
        let i_f256 = f256::Additive(i).to_multiplier();
        let i_f2e16 = f2e16::Additive(i as u16).to_multiplier();
        for j in 0..=255 {
            let j_f256 = f256::Additive(j).mul(i_f256);
            let j_f2e16 = f2e16::Additive(j as u16).mul(i_f2e16);
            assert_eq!(j_f256.0 as u16, j_f2e16.0);
        }
    }    
}

pub mod novel_poly_basis;
#[cfg(feature = "cmp-with-cxx")]
pub mod novel_poly_basis_cxx;

pub const N_VALIDATORS: usize = 2000;

pub const BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rand_data.bin"));

/// Assert the byte ranges derived from the index vec are recovered properly
pub fn assert_recovery(payload: &[u8], reconstructed_payload: &[u8], dropped_indices: IndexVec) {
	assert!(reconstructed_payload.len() >= payload.len());

	dropped_indices.into_iter().for_each(|dropped_idx| {
		let byteoffset = dropped_idx * 2;
		let range = byteoffset..(byteoffset + 2);
		// dropped indices are over `n`, but our data indices are just of length `k * 2`
		if payload.len() >= range.end {
			assert_eq!(
				&payload[range.clone()],
				&reconstructed_payload[range.clone()],
				"Data at bytes {:?} must match:",
				range
			);
		}
	});
}

pub fn drop_random_max(shards: &mut [Option<WrappedShard>], n: usize, k: usize, rng: &mut impl rand::Rng) -> IndexVec {
	let l = shards.len();
	let already_dropped = n.saturating_sub(l);
	let iv = rand::seq::index::sample(rng, l, n - k - already_dropped);
	assert_eq!(iv.len(), n - k);
	iv.clone().into_iter().for_each(|idx| {
		shards[idx] = None;
	});
	let kept_count = shards.iter().map(Option::is_some).count();
	assert!(kept_count >= k);
	iv
}

pub fn roundtrip<E, R>(encode: E, reconstruct: R, payload: &[u8], real_n: usize) -> Result<()>
where
	E: for<'r> Fn(&'r [u8], usize) -> Result<Vec<WrappedShard>>,
	R: Fn(Vec<Option<WrappedShard>>, usize) -> Result<Vec<u8>>,
{
	let v =
		roundtrip_w_drop_closure::<E, R, _, SmallRng>(encode, reconstruct, payload, real_n, drop_random_max)?;
	Ok(v)
}

pub fn roundtrip_w_drop_closure<E, R, F, G>(
	encode: E,
	reconstruct: R,
	payload: &[u8],
	real_n: usize,
	mut drop_rand: F,
) -> Result<()>
where
	E: for<'r> Fn(&'r [u8], usize) -> Result<Vec<WrappedShard>>,
	R: Fn(Vec<Option<WrappedShard>>, usize) -> Result<Vec<u8>>,
	F: for<'z> FnMut(&'z mut [Option<WrappedShard>], usize, usize, &mut G) -> IndexVec,
	G: rand::Rng + rand::SeedableRng<Seed = [u8; 32]>,
{
	let mut rng = <G as rand::SeedableRng>::from_seed(SMALL_RNG_SEED);

	// Construct the shards
	let shards = encode(payload, real_n)?;

	// Make a copy and transform it into option shards arrangement
	// for feeding into reconstruct_shards
	let mut received_shards = shards.into_iter().map(Some).collect::<Vec<Option<WrappedShard>>>();

	let dropped_indices = drop_rand(received_shards.as_mut_slice(), real_n, real_n / 3, &mut rng);

	let recovered_payload = reconstruct(received_shards, real_n)?;

	assert_recovery(&payload[..], &recovered_payload[..], dropped_indices);
	Ok(())
}

#[cfg(test)]
mod test {
	use super::*;

	#[cfg(feature = "status-quo")]
	#[test]
	fn status_quo_roundtrip() -> Result<()> {
		roundtrip(status_quo::encode, status_quo::reconstruct, &BYTES[..1337], N_VALIDATORS)
	}

	#[test]
	fn novel_poly_basis_roundtrip() -> Result<()> {
		roundtrip(novel_poly_basis::encode, novel_poly_basis::reconstruct, &BYTES[..1337], N_VALIDATORS)
	}
}
