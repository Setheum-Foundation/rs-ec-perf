// Encoding/erasure decoding for Reed-Solomon codes over binary extension fields
//
// Derived impl of `RSAErasureCode.c`.
//
// Lin, Han and Chung, "Novel Polynomial Basis and Its Application to Reed-Solomon Erasure Codes," FOCS14.
// (http://arxiv.org/abs/1404.3458)

use super::*;

use core::mem::transmute;

use std::{cmp, mem::{self, transmute_copy}, ops::{AddAssign, ShrAssign}, slice::from_raw_parts};

type GFSymbol = u16;

const FIELD_BITS: usize = 16;

const GENERATOR: GFSymbol = 0x2D; //x^16 + x^5 + x^3 + x^2 + 1

//Cantor basis
const BASE: [GFSymbol; FIELD_BITS] =
	[1_u16, 44234, 15374, 5694, 50562, 60718, 37196, 16402, 27800, 4312, 27250, 47360, 64952, 64308, 65336, 39198];

const FIELD_SIZE: usize = 1_usize << FIELD_BITS;

const MODULO: GFSymbol = (FIELD_SIZE - 1) as GFSymbol;

static mut LOG_TABLE: [GFSymbol; FIELD_SIZE] = [0_u16; FIELD_SIZE];
static mut EXP_TABLE: [GFSymbol; FIELD_SIZE] = [0_u16; FIELD_SIZE];

//-----Used in decoding procedure-------
//twisted factors used in FFT
static mut SKEW_FACTOR: [GFSymbol; MODULO as usize] = [0_u16; MODULO as usize];

//factors used in formal derivative
static mut B: [GFSymbol; FIELD_SIZE >> 1] = [0_u16; FIELD_SIZE >> 1];

//factors used in the evaluation of the error locator polynomial
static mut LOG_WALSH: [GFSymbol; FIELD_SIZE] = [0_u16; FIELD_SIZE];

//return a*EXP_TABLE[b] over GF(2^r)
fn mul_table(a: GFSymbol, b: GFSymbol) -> GFSymbol {
	if a != 0_u16 {
		unsafe {
			let offset = (LOG_TABLE[a as usize] as u32 + b as u32 & MODULO as u32)
				+ (LOG_TABLE[a as usize] as u32 + b as u32 >> FIELD_BITS);
			EXP_TABLE[offset as usize]
		}
	} else {
		0_u16
	}
}

const fn log2(mut x: usize) -> usize {
	let mut o: usize = 0;
	while x > 1 {
		x >>= 1;
		o += 1;
	}
	o
}

const fn is_power_of_2(x: usize) -> bool {
	return x > 0_usize && x & (x - 1) == 0;
}


//fast Walsh–Hadamard transform over modulo mod
fn walsh(data: &mut [GFSymbol], size: usize) {
	let mut depart_no = 1_usize;
	while depart_no < size {
		let mut j = 0;
		while j < size {
			for i in j..(depart_no + j) {
				let tmp2: u32 = data[i] as u32 + MODULO as u32 - data[i + depart_no] as u32;
				data[i] = ((data[i] as u32 + data[i + depart_no] as u32 & MODULO as u32)
					+ (data[i] as u32 + data[i + depart_no] as u32 >> FIELD_BITS)) as GFSymbol;
				data[i + depart_no] = ((tmp2 & MODULO as u32) + (tmp2 >> FIELD_BITS)) as GFSymbol;
			}
			j += depart_no << 1;
		}
		depart_no <<= 1;
	}
}

//formal derivative of polynomial in the new basis
fn formal_derivative(cos: &mut [GFSymbol], size: usize) {
	for i in 1..size {
		let length = ((i ^ i - 1) + 1) >> 1;
		for j in (i - length)..i {
			cos[j] ^= cos.get(j + length).copied().unwrap_or_default();
		}
	}
	let mut i = size;
	while i < FIELD_SIZE && i < cos.len() {
		for j in 0..size {
			cos[j] ^= cos.get(j + i).copied().unwrap_or_default();
		}
		i <<= 1;
	}
}

// We want the low rate scheme given in
// https://www.citi.sinica.edu.tw/papers/whc/5524-F.pdf
// and https://github.com/catid/leopard/blob/master/docs/LowRateDecoder.pdf
// but this code resembles https://github.com/catid/leopard which 
// implements the high rate decoder in 
// https://github.com/catid/leopard/blob/master/docs/HighRateDecoder.pdf
// We're hunting for the differences and trying to undersrtand the algorithm.

//IFFT in the proposed basis
fn inverse_fft_in_novel_poly_basis(data: &mut [GFSymbol], size: usize, index: usize) {
	// All line references to Algorithm 2 page 6288 of
	// https://www.citi.sinica.edu.tw/papers/whc/5524-F.pdf

	// Depth of the recursion on line 7 and 8 is given by depart_no aka 1 << (i of Algorithm 2).
	let mut depart_no = 1_usize;
	while depart_no < size {
		// Bredth first loop across recursions from line 7 and 8, so
		// this j indicates recusion branch, presumably making this j be
		// r in Algorith 2 and increases by depart_no gives powers of two.
		// Q:  Is j shifted from r any?
		let mut j = depart_no;
		while j < size {
			// Loop on line 3, so i corresponds to j in Algorithm 2
			for i in (j - depart_no)..j {
				// Line 4, justified by (34) page 6288, but
				// adding depart_no acts like the r+2^i superscript.
				data[i + depart_no] ^= data[i];
			}

			// TODO: Unclear how skew does not depend upon i, maybe the s_i is constant?
			// Or maybe this craetes a problem?	 Non-constant skew yields an invertable
			// map, but maybe not an FFT.
			let skew = unsafe { SKEW_FACTOR[j + index - 1] };
			if skew != MODULO {
				// Again loop on line 3, except skew should depend upon i aka j in Algorithm 2 (TODO)
				for i in (j - depart_no)..j {
					// Line 5, justified by (35) page 6288, but
					// adding depart_no acts like the r+2^i superscript.
					data[i] ^= mul_table(data[i + depart_no], skew);
				}
			}

			// Increment by double depart_no in agreement with
			// our updating 2*depart_no elements at this depth.
			j += depart_no << 1;
		}
		depart_no <<= 1;
	}
}

//FFT in the proposed basis
fn fft_in_novel_poly_basis(data: &mut [GFSymbol], size: usize, index: usize) {
	// All line references to Algorithm 1 page 6287 of 
	// https://www.citi.sinica.edu.tw/papers/whc/5524-F.pdf

	// Depth of the recursion on line 3 and 4 is given by depart_no aka 1 << (i of Algorithm 1).
	let mut depart_no = size >> 1_usize;
	while depart_no > 0 {
		// Bredth first loop across recursions from line 3 and 4, so
		// this j indicates recusion branch, presumably making this j be
		// somewhat like r in Algorith 1, in that it increases by depart_no.
		let mut j = depart_no;
		while j < size {
			// TODO: Unclear how skew does not depend upon i, maybe the s_i is constant?
			// Or maybe this craetes a problem?	 Non-constant skew yields an invertable
			// map, but maybe not an FFT.

			// They index the skew in line 6 aka (28) page 6287 by i and j but not by r,
			// so here we index the skew by 
			let skew = unsafe { SKEW_FACTOR[j + index - 1] };
			if skew != MODULO {
				// Loop on line 5, except skew should depend upon i aka j in Algorithm 1 (TODO)
				for i in (j - depart_no)..j {
					// Line 6, explained by (28) page 6287, but
					// adding depart_no acts like the r+2^i superscript.
					data[i] ^= mul_table(data[i + depart_no], skew);
				}
			}

			// Again loop on line 5, so i corresponds to j in Algorithm 1
			for i in (j - depart_no)..j {
				// Line 7, explained by (31) page 6287, but
				// adding depart_no acts like the r+2^i superscript.
				data[i + depart_no] ^= data[i];
			}

			// Increment by double depart_no in agreement with
			// our updating 2*depart_no elements at this depth.
			j += depart_no << 1;
		}
		depart_no >>= 1;
	}
	return;
}

//initialize LOG_TABLE[], EXP_TABLE[]
unsafe fn init() {
	let mas: GFSymbol = (1 << FIELD_BITS - 1) - 1;
	let mut state: usize = 1;
	for i in 0_usize..(MODULO as usize) {
		EXP_TABLE[state] = i as GFSymbol;
		if (state >> FIELD_BITS - 1) != 0 {
			state &= mas as usize;
			state = state << 1_usize ^ GENERATOR as usize;
		} else {
			state <<= 1;
		}
	}
	EXP_TABLE[0] = MODULO;

	LOG_TABLE[0] = 0;
	for i in 0..FIELD_BITS {
		for j in 0..(1 << i) {
			LOG_TABLE[j + (1 << i)] = LOG_TABLE[j] ^ BASE[i];
		}
	}
	for i in 0..FIELD_SIZE {
		LOG_TABLE[i] = EXP_TABLE[LOG_TABLE[i] as usize];
	}

	for i in 0..FIELD_SIZE {
		EXP_TABLE[LOG_TABLE[i] as usize] = i as GFSymbol;
	}
	EXP_TABLE[MODULO as usize] = EXP_TABLE[0];
}

//initialize SKEW_FACTOR[], B[], LOG_WALSH[]
unsafe fn init_dec() {
	let mut field_base: [GFSymbol; FIELD_BITS - 1] = Default::default();

	for i in 1..FIELD_BITS {
		field_base[i - 1] = 1 << i;
	}

	// 
	for m in 0..(FIELD_BITS - 1) {
		let step = 1 << (m + 1);
		SKEW_FACTOR[(1 << m) - 1] = 0;
		for i in m..(FIELD_BITS - 1) {
			let s = 1 << (i + 1);

			let mut j = (1 << m) - 1;
			while j < s {
				// Justified by (5) page 6285
				SKEW_FACTOR[j + s] = SKEW_FACTOR[j] ^ field_base[i];
				j += step;
			}
		}

		let idx = mul_table(field_base[m], LOG_TABLE[(field_base[m] ^ 1_u16) as usize]);
		field_base[m] = MODULO - LOG_TABLE[idx as usize];

		for i in (m + 1)..(FIELD_BITS - 1) {
			let b = LOG_TABLE[(field_base[i] as u16 ^ 1_u16) as usize] as u32 + field_base[m] as u32;
			let b = b % MODULO as u32;
			field_base[i] = mul_table(field_base[i], b as u16);
		}
	}
	// 
	for i in 0..(MODULO as usize) {
		SKEW_FACTOR[i] = LOG_TABLE[SKEW_FACTOR[i] as usize];
	}

	field_base[0] = MODULO - field_base[0];
	for i in 1..(FIELD_BITS - 1) {
		field_base[i] = ((MODULO as u32 - field_base[i] as u32 + field_base[i - 1] as u32) % MODULO as u32) as GFSymbol;
	}

	B[0] = 0;
	for i in 0..(FIELD_BITS - 1) {
		let depart = 1 << i;
		for j in 0..depart {
			B[j + depart] = ((B[j] as u32 + field_base[i] as u32) % MODULO as u32) as GFSymbol;
		}
	}

	mem_cpy(&mut LOG_WALSH[..], &LOG_TABLE[..]);
	LOG_WALSH[0] = 0;
	walsh(&mut LOG_WALSH[..], FIELD_SIZE);
}

//Encoding alg for k/n < 0.5: message is a power of two
fn encode_low(data: &[GFSymbol], k: usize, codeword: &mut [GFSymbol], n: usize) {
	assert!(k + k <	 n);
	assert_eq!(codeword.len(), n);
	assert_eq!(data.len(), n);

	mem_cpy(&mut codeword[0..k], &data[0..k]);

	inverse_fft_in_novel_poly_basis(codeword, k, 0);

	let (first_k, skip_first_k) = codeword.split_at_mut(k);
	let mut i = k;
	while i < n {
		let s = i - k;
		mem_cpy(&mut skip_first_k[s..i], first_k);
		fft_in_novel_poly_basis(&mut skip_first_k[s..i], k, i);
		i += k;
	}

	mem_cpy(&mut codeword[0..k], &data[0..k]);
}

fn mem_zero(zerome: &mut [GFSymbol]) {
	for i in 0..zerome.len() {
		zerome[i] = 0_u16;
	}
}

fn mem_cpy(dest: &mut [GFSymbol], src: &[GFSymbol]) {
	let sl = src.len();
	debug_assert_eq!(dest.len(), sl);
	for i in 0..sl {
		dest[i] = src[i];
	}
}

//data: message array. parity: parity array. mem: buffer(size>= n-k)
//Encoding alg for k/n>0.5: parity is a power of two.
fn encode_high(data: &[GFSymbol], k: usize, parity: &mut [GFSymbol], mem: &mut [GFSymbol], n: usize) {
	let t: usize = n - k;

	mem_zero(&mut parity[0..t]);

	let mut i = t;
	while i < n {
		mem_cpy(&mut mem[..t], &data[(i - t)..t]);

		inverse_fft_in_novel_poly_basis(mem, t, i);
		for j in 0..t {
			parity[j] ^= mem[j];
		}
		i += t;
	}
	fft_in_novel_poly_basis(parity, t, 0);
}

//Compute the evaluations of the error locator polynomial
fn decode_init(erasure: &[bool], log_walsh2: &mut [GFSymbol], n: usize) {
	for i in 0..n {
		log_walsh2[i] = erasure[i] as u16;
	}
	walsh(log_walsh2, n);
	for i in 0..n {
		log_walsh2[i] = (log_walsh2[i] as usize * unsafe { LOG_WALSH[i] } as usize % MODULO as usize) as GFSymbol;
	}
	walsh(log_walsh2, n);
	for i in 0..n {
		if erasure[i] {
			log_walsh2[i] = MODULO - log_walsh2[i];
		}
	}
}

fn decode_main(codeword: &mut [GFSymbol], k: usize, erasure: &[bool], log_walsh2: &[GFSymbol], n: usize) {
	assert!(codeword.len() >= K);
	assert_eq!(codeword.len(), n);
	assert!(erasure.len() >= k);
	assert_eq!(erasure.len(), n);

	// technically we only need to recover
	// the first `k` instead of all `n` which
	// would include parity chunks.
	let recover_up_to = n;
	for i in 0..recover_up_to {
		codeword[i] = if !erasure[i] {
			mul_table(codeword[i], log_walsh2[i])
		} else {
			0_u16
		};
	}
	inverse_fft_in_novel_poly_basis(codeword, n, 0);

	//formal derivative
	let mut i = 0;
	while i < n {
		let b = unsafe { B[i >> 1] };
		codeword[i] = mul_table(codeword[i], MODULO - b);
		codeword[i + 1] = mul_table(codeword[i + 1], MODULO - b);
		i += 2;
	}
	formal_derivative(codeword, n);
	let mut i = 0;
	while i < k {
		let b = unsafe { B[i >> 1] };
		codeword[i] = mul_table(codeword[i], b);
		codeword[i + 1] = mul_table(codeword[i + 1], b);
		i += 2;
	}

	fft_in_novel_poly_basis(codeword, recover_up_to, 0);
	for i in 0..recover_up_to {
		codeword[i] = if erasure[i] { mul_table(codeword[i], log_walsh2[i]) } else { 0_u16 };
	}
}


const N: usize = crate::N_VALIDATORS;
const K: usize = crate::DATA_SHARDS;

use itertools::Itertools;
use mem::zeroed;

pub fn encode(data: &[u8]) -> Vec<WrappedShard> {
	unsafe { init() };

	// must be power of 2
	let l = log2(data.len());
	let l = 1 << l;
	let l = if l >= data.len() {
		l
	} else {
		l << 1
	};
	assert!(l >= data.len());
	assert!(is_power_of_2(l));
	assert!(is_power_of_2(N), "Algorithm only works for 2^m sizes for N");
	assert!(is_power_of_2(K), "Algorithm only works for 2^m sizes for K");


	// pad the incoming data with trailing 0s
	let zero_bytes_to_add = dbg!(l) - dbg!(data.len());
	let mut data: Vec<GFSymbol> = data.into_iter().copied().chain(
		std::iter::repeat(0u8).take(zero_bytes_to_add)
	)
		.tuple_windows()
		.step_by(2)
		.map(|(a,b)| { (b as u16) << 8 | a as u16 })
		.collect::<Vec<GFSymbol>>();

	// assert_eq!(K, data.len());
	assert_eq!(data.len() * 2, l + zero_bytes_to_add);

	// two bytes make one `l / 2`
	let l = l / 2;
	assert_eq!(l, N, "For now we only want to test of variants that don't have to be 0 padded");
	let mut codeword = data.clone();
	assert_eq!(codeword.len(), N);

	if K + K > N {
		let (data_till_t, data_skip_t) = data.split_at_mut(N - K);
		encode_high(data_skip_t, K, data_till_t, &mut codeword[..], N);
	} else {
		encode_low(&data[..], K, &mut codeword[..], N);
	}

	mem_cpy(&mut codeword[..], &data[..]);

	println!("Codeword:");
	for i in 0..N {
		print!("{:04x} ", codeword[i]);
	}
	println!("");

	// XXX currently this is only done for one codeword!

	let shards = (0..N).into_iter().map(|i| {
		WrappedShard::new({
			let arr = codeword[i].to_le_bytes();
			arr.to_vec()
		}
		)
	})
	.collect::<Vec<WrappedShard>>();

	shards
}

pub fn reconstruct(received_shards: Vec<Option<WrappedShard>>) -> Option<Vec<u8>> {

	unsafe { init_dec() };

	// collect all `None` values
	let mut erased_count = 0;
	let erasures = received_shards
		.iter()
		.map(|x| x.is_none())
		.inspect(|v| { if *v {
			erased_count += 1;
		}})
		.collect::<Vec<bool>>();

	// The recovered _data_ chunks AND parity chunks
	let mut recovered: Vec<u16> = std::iter::repeat(0u16).take(N).collect();

	// get rid of all `None`s
	let mut codeword = received_shards.into_iter()
		.enumerate()
		.map(|(idx, wrapped)| {
			// fill the gaps with `0_u16` codewords
			if let Some(wrapped) = wrapped {
				let v: &[[u8; 2]] = wrapped.as_ref();
				(idx, u16::from_le_bytes(v[0]))
			} else {
				(idx, 0_u16)
			}
		})
		.map(|(idx, codeword)| {
			// copy the good messages (here it's just one codeword/u16 right now)
			if idx < N {
				recovered[idx] = codeword;
			}
			codeword
		})
		.collect::<Vec<u16>>();

	// filled up the remaining spots with 0s
	// XXX TODO now all valid codewords are in the front, which
	// XXX is not what we want, since decode_main overwrites
	// XXX the erase portions
	assert_eq!(codeword.len(), N);

	let k = K; //N - erased_count;

	//---------Erasure decoding----------------
	let mut log_walsh2: [GFSymbol; N] = [0_u16; N];
	//Evaluate error locator polynomial
	decode_init(&erasures[..], &mut log_walsh2[..], N);
	//---------main processing----------
	decode_main(&mut codeword[..], k, &erasures[..], &log_walsh2[..], N);

	println!("Decoded result:");
	for idx in 0..N {
		if erasures[idx] {
			print!("{:04x} ", codeword[idx]);
			recovered[idx] = codeword[idx];
		} else {
			print!("XXXX ");
		};
	}

	let recovered = unsafe {
		// TODO assure this does not leak memory
		let x = from_raw_parts(recovered.as_ptr() as *const u8, recovered.len() * 2);
		std::mem::forget(recovered);
		x
	};
	Some(recovered.to_vec())
}

#[cfg(test)]
mod test {
	use super::*;

	/// Generate a random index
	fn rand_gf_element() -> GFSymbol {
		use rand::distributions::{Distribution, Uniform};
		use rand::thread_rng;

		let mut rng = thread_rng();
		let uni = Uniform::<GFSymbol>::new_inclusive(0, MODULO);
		uni.sample(&mut rng)
	}

	#[test]
	fn ported_c_test() {
		unsafe {
			init(); //fill log table and exp table
			init_dec(); //compute factors used in erasure decoder
		}

		//-----------Generating message----------
		//message array
		let mut data: [GFSymbol; N] = [0; N];

		for i in (N - K)..N {
			//filled with random numbers
			data[i] = rand_gf_element();
		}

		assert_eq!(data.len(), N);

		println!("Message(First n-k are zeros): ");
		for i in 0..N {
			print!("{:04x} ", data[i]);
		}
		println!("");

		//---------encoding----------
		let mut codeword = [0_u16; N];

		if K + K > N {
			let (data_till_t, data_skip_t) = data.split_at_mut(N - K);
			encode_high(data_skip_t, K, data_till_t, &mut codeword[..], N);
		} else {
			encode_low(&data[..], K, &mut codeword[..], N);
		}

		mem_cpy(&mut codeword[..], &data[..]);

		println!("Codeword:");
		for i in 0..N {
			print!("{:04x} ", codeword[i]);
		}
		println!("");

		//--------erasure simulation---------

		//Array indicating erasures
		let mut erasure: [bool; N] = [false; N];
		for i in K..N {
			erasure[i] = true;
		}

		//permuting the erasure array
		{
			let mut i = N - 1;
			while i > 0 {
				let pos: usize = rand_gf_element() as usize % (i + 1);
				if i != pos {
					erasure.swap(i, pos);
				}
				i -= 1;
			}

			for i in 0..N {
				//erasure codeword symbols
				if erasure[i] {
					codeword[i] = 0 as GFSymbol;
				}
			}
		}

		println!("Erasure (XXXX is erasure):");
		for i in 0..N {
			if erasure[i] {
				print!("XXXX ");
			} else {
				print!("{:04x} ", codeword[i]);
			}
		}
		println!("");

		//---------Erasure decoding----------------
		let mut log_walsh2: [GFSymbol; N] = [0_u16; N];
		decode_init(&erasure[..], &mut log_walsh2[..], N); //Evaluate error locator polynomial
												   //---------main processing----------
		decode_main(&mut codeword[..], K, &erasure[..], &log_walsh2[..], N);

		println!("Decoded result:");
		for i in 0..N {
			if erasure[i] {
				print!("{:04x} ", codeword[i]);
			} else {
				print!("XXXX ");
			};
		}
		println!("");

		for i in 0..N {
			//Check the correctness of the result
			if data[i] != codeword[i] {
				panic!("Decoding Error! value at [{}] should={:04x} vs is={:04x}", i, data[i], codeword[i]);
			}
		}
		println!("Decoding is successful!");
	}
}
