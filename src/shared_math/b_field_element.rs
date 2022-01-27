use super::other;
use super::traits::{FromVecu8, GetPrimitiveRootOfUnity, Inverse};
use crate::shared_math::traits::GetRandomElements;
use crate::shared_math::traits::{
    CyclicGroupGenerator, IdentityValues, ModPowU32, ModPowU64, New, PrimeField,
};
use crate::utils::FIRST_THOUSAND_PRIMES;
use num_traits::{One, Zero};
use phf::phf_map;
use rand::prelude::ThreadRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::ops::{AddAssign, MulAssign, SubAssign};
use std::{
    fmt::{self},
    ops::{Add, Div, Mul, Neg, Rem, Sub},
};

static PRIMITIVE_ROOTS: phf::Map<u64, u128> = phf_map! {
    2u64 => 18446744069414584320,
    4u64 => 281474976710656,
    8u64 => 18446744069397807105,
    16u64 => 17293822564807737345,
    32u64 => 70368744161280,
    64u64 => 549755813888,
    128u64 => 17870292113338400769,
    256u64 => 13797081185216407910,
    512u64 => 1803076106186727246,
    1024u64 => 11353340290879379826,
    2048u64 => 455906449640507599,
    4096u64 => 17492915097719143606,
    8192u64 => 1532612707718625687,
    16384u64 => 16207902636198568418,
    32768u64 => 17776499369601055404,
    65536u64 => 6115771955107415310,
    131072u64 => 12380578893860276750,
    262144u64 => 9306717745644682924,
    524288u64 => 18146160046829613826,
    1048576u64 => 3511170319078647661,
    2097152u64 => 17654865857378133588,
    4194304u64 => 5416168637041100469,
    8388608u64 => 16905767614792059275,
    16777216u64 => 9713644485405565297,
    33554432u64 => 5456943929260765144,
    67108864u64 => 17096174751763063430,
    134217728u64 => 1213594585890690845,
    268435456u64 => 6414415596519834757,
    536870912u64 => 16116352524544190054,
    1073741824u64 => 9123114210336311365,
    2147483648u64 => 4614640910117430873,
    4294967296u64 => 1753635133440165772,
};

// BFieldElement ∈ ℤ_{2^64 - 2^32 + 1}
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash, Serialize, Deserialize)]
pub struct BFieldElement(u128);

impl BFieldElement {
    pub const QUOTIENT: u128 = 0xffff_ffff_0000_0001u128; // 2^64 - 2^32 + 1
    pub const MAX: u128 = Self::QUOTIENT - 1;

    pub fn new(value: u128) -> Self {
        Self(value % Self::QUOTIENT)
    }

    pub fn increment(&mut self) {
        self.0 = (self.0 + 1) % Self::QUOTIENT;
    }

    pub fn decrement(&mut self) {
        self.0 = (Self::MAX + self.0) % Self::QUOTIENT
    }

    // TODO: Use Rust Pow. TODO: Maybe move this out into a library along with xgcd().
    // TODO: Name this collection of traits as something like... FieldElementInternalNumRepresentation
    fn mod_pow_raw(&self, pow: u64) -> u128 {
        // Special case for handling 0^0 = 1
        if pow == 0 {
            return 1u128;
        }

        let mut acc: u128 = 1;
        let mod_value: u128 = Self::QUOTIENT;
        let res = self.0;

        for i in 0..64 {
            acc = acc * acc % mod_value;
            let set: bool = pow & (1 << (64 - 1 - i)) != 0;
            if set {
                acc = acc * res % mod_value;
            }
        }

        acc
    }

    // TODO: Currently, IdentityValues has &self as part of its signature, so we hotfix
    // being able to refer to a zero/one element without having an element at hand. This
    // will go away when moving to Zero/One traits.
    pub fn ring_zero() -> Self {
        Self(0)
    }

    pub fn ring_one() -> Self {
        Self(1)
    }

    #[must_use]
    pub fn mod_pow(&self, pow: u64) -> Self {
        Self(self.mod_pow_raw(pow))
    }

    // TODO: Abstract for both i128 and u128 so we don't keep multiple copies of the same algorithm.
    fn primes_lt(bound: u128) -> Vec<u128> {
        let mut primes: Vec<bool> = (0..bound + 1).map(|num| num == 2 || num & 1 != 0).collect();
        let mut num = 3u128;
        while num * num <= bound {
            let mut j = num * num;
            while j <= bound {
                primes[j as usize] = false;
                j += num;
            }
            num += 2;
        }
        primes
            .into_iter()
            .enumerate()
            .skip(2)
            .filter_map(|(i, p)| if p { Some(i as u128) } else { None })
            .collect::<Vec<u128>>()
    }

    pub fn legendre_symbol(&self) -> i8 {
        let elem = self.mod_pow((Self::QUOTIENT - 1) as u64 / 2).0;

        // Ugly hack to force a result in {-1,0,1}
        if elem == Self::QUOTIENT - 1 {
            -1
        } else if elem == 0 {
            0
        } else {
            1
        }
    }
}

impl fmt::Display for BFieldElement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Inverse for BFieldElement {
    #[must_use]
    fn inverse(&self) -> Self {
        let (_, _, a) = other::xgcd(Self::QUOTIENT as i128, self.0 as i128);

        Self(
            ((a % Self::QUOTIENT as i128 + Self::QUOTIENT as i128) % Self::QUOTIENT as i128)
                as u128,
        )
    }
}

impl ModPowU32 for BFieldElement {
    fn mod_pow_u32(&self, exp: u32) -> Self {
        // TODO: This can be sped up by a factor 2 by implementing
        // it for u32 and not using the 64-bit version
        self.mod_pow(exp as u64)
    }
}

impl CyclicGroupGenerator for BFieldElement {
    fn get_cyclic_group_elements(&self, max: Option<usize>) -> Vec<Self> {
        let mut val = *self;
        let mut ret: Vec<Self> = vec![self.ring_one()];

        loop {
            ret.push(val);
            val *= *self;
            if val.is_one() || max.is_some() && ret.len() >= max.unwrap() {
                break;
            }
        }
        ret
    }
}

impl GetRandomElements for BFieldElement {
    fn random_elements(length: usize, prng: &mut ThreadRng) -> Vec<Self> {
        let mut values: Vec<BFieldElement> = Vec::with_capacity(length);
        let max = BFieldElement::MAX as u64;

        while values.len() < length {
            let n = prng.next_u64();

            if n > max {
                continue;
            }

            values.push(BFieldElement::new(n as u128));
        }

        values
    }
}

impl New for BFieldElement {
    fn new_from_usize(&self, value: usize) -> Self {
        Self::new(value as u128)
    }
}

// This is used for: Convert a hash value to a BFieldElement. Consider making From<Blake3Hash> trait
impl FromVecu8 for BFieldElement {
    fn from_vecu8(&self, bytes: Vec<u8>) -> Self {
        // TODO: Right now we only accept if 'bytes' has 8 bytes; while that is true in
        // the single call site this is used, it also seems unnecessarily fragile (when we
        // change from BLAKE3 to Rescue-Prime, the hash length will change and this will be
        // be wrong). We should make this a From<Blake3Hash> to ensure that it has the right
        // length.
        let (eight_bytes, _rest) = bytes.as_slice().split_at(std::mem::size_of::<u64>());
        let coerced: [u8; 8] = eight_bytes.try_into().unwrap();
        let n: u64 = u64::from_ne_bytes(coerced);
        BFieldElement::new(n as u128)
    }
}

impl PrimeField for BFieldElement {
    type Elem = BFieldElement;
}

// TODO: We implement IdentityValues so this module works in existing code, but we want to replace IdentityValues with Zero and One eventually.
// impl Zero for BFieldElement {
//     fn zero() -> Self {
//         BFieldElement(0)
//     }

//     fn is_zero(&self) -> bool {
//         self.0 == 0
//     }
// }

// impl One for BFieldElement {
//     fn one() -> Self {
//         BFieldElement(1)
//     }

//     fn is_one(&self) -> bool {
//         self.0 == 1
//     }
// }

impl IdentityValues for BFieldElement {
    fn is_zero(&self) -> bool {
        self.0 == 0
    }

    fn is_one(&self) -> bool {
        self.0 == 1
    }

    fn ring_zero(&self) -> Self {
        BFieldElement(0)
    }

    fn ring_one(&self) -> Self {
        BFieldElement(1)
    }
}

impl Add for BFieldElement {
    type Output = Self;

    #[inline]
    fn add(self, other: Self) -> Self {
        Self((self.0 + other.0) % Self::QUOTIENT)
    }
}

impl AddAssign for BFieldElement {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 = (self.0 + rhs.0) % Self::QUOTIENT;
    }
}

impl SubAssign for BFieldElement {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = (Self::QUOTIENT - rhs.0 + self.0) % Self::QUOTIENT;
    }
}

impl MulAssign for BFieldElement {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        self.0 = (self.0 * rhs.0) % Self::QUOTIENT;
    }
}

impl Mul for BFieldElement {
    type Output = Self;

    #[inline]
    fn mul(self, other: Self) -> Self {
        Self((self.0 * other.0) % Self::QUOTIENT)
    }
}

impl Neg for BFieldElement {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self {
        Self(Self::QUOTIENT - self.0)
    }
}

impl Rem for BFieldElement {
    type Output = Self;

    fn rem(self, other: Self) -> Self {
        Self(self.0 % other.0)
    }
}

impl Sub for BFieldElement {
    type Output = Self;

    #[inline]
    fn sub(self, other: Self) -> Self {
        -other + self
    }
}

impl Div for BFieldElement {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        Self((other.inverse().0 * self.0) % Self::QUOTIENT)
    }
}

// TODO: We probably wanna make use of Rust's Pow, but for now we copy from ...big:
impl ModPowU64 for BFieldElement {
    fn mod_pow_u64(&self, pow: u64) -> Self {
        self.mod_pow(pow)
    }
}

impl GetPrimitiveRootOfUnity for BFieldElement {
    fn get_primitive_root_of_unity(&self, n: u128) -> (Option<BFieldElement>, Vec<u128>) {
        // Check if n is one of the values for which we have pre-calculated roots
        if PRIMITIVE_ROOTS.contains_key(&(n as u64)) {
            return (
                Some(BFieldElement::new(PRIMITIVE_ROOTS[&(n as u64)])),
                vec![2],
            );
        }

        let mut primes: Vec<u128> = vec![];

        if n <= 1 {
            return (Some(BFieldElement::ring_one()), primes);
        }

        // Calculate prime factorization of n
        // Check if n = 2^k
        if n & (n - 1) == 0 {
            primes = vec![2];
        } else {
            let mut m = n;
            for prime in FIRST_THOUSAND_PRIMES.iter().map(|&p| p as u128) {
                if m == 1 {
                    break;
                }
                if m % prime == 0 {
                    primes.push(prime);
                    while m % prime == 0 {
                        m /= prime;
                    }
                }
            }
            // This might be prohibitively expensive
            if m > 1 {
                let mut other_primes = BFieldElement::primes_lt(m)
                    .into_iter()
                    .filter(|&x| n % x == 0)
                    .collect();
                primes.append(&mut other_primes);
            }
        };

        // N must divide the field prime minus one for a primitive nth root of unity to exist
        if !((Self::QUOTIENT - 1) % n).is_zero() {
            return (None, primes);
        }

        let mut primitive_root: Option<BFieldElement> = None;
        let mut candidate: BFieldElement = BFieldElement::ring_one();
        #[allow(clippy::suspicious_operation_groupings)]
        while primitive_root == None && candidate.0 < Self::QUOTIENT {
            if (-candidate.legendre_symbol()).is_one()
                && primes.iter().filter(|&x| n % x == 0).all(|x| {
                    !candidate
                        .mod_pow_raw(((Self::QUOTIENT - 1) / x) as u64)
                        .is_one()
                })
            {
                primitive_root = Some(candidate.mod_pow(((Self::QUOTIENT - 1) / n) as u64));
            }

            candidate.0 += 1;
        }

        (primitive_root, primes)
    }
}

#[cfg(test)]
mod b_prime_field_element_test {
    use crate::{
        shared_math::{b_field_element::*, polynomial::Polynomial},
        utils::generate_random_numbers,
    };
    use itertools::izip;
    use proptest::prelude::*;

    // TODO: Move this into separate file.
    macro_rules! bfield_elem {
        ($value:expr) => {{
            BFieldElement::new($value)
        }};
    }

    #[test]
    fn test_zero_one() {
        let zero = bfield_elem!(0);
        let one = bfield_elem!(1);

        assert!(zero.is_zero());
        assert!(!zero.is_one());
        assert!(!one.is_zero());
        assert!(one.is_one());
        assert!(bfield_elem!(BFieldElement::MAX + 1).is_zero());
        assert!(bfield_elem!(BFieldElement::MAX + 2).is_one());
    }

    #[test]
    fn increment_and_decrement_test() {
        let mut val_a = bfield_elem!(0);
        let mut val_b = bfield_elem!(1);
        let mut val_c = bfield_elem!(BFieldElement::MAX - 1);
        let max = BFieldElement::new(BFieldElement::MAX);
        val_a.increment();
        assert!(val_a.is_one());
        val_b.increment();
        assert!(!val_b.is_one());
        assert_eq!(BFieldElement::new(2), val_b);
        val_b.increment();
        assert_eq!(BFieldElement::new(3), val_b);
        assert_ne!(max, val_c);
        val_c.increment();
        assert_eq!(max, val_c);
        val_c.increment();
        assert!(val_c.is_zero());
        val_c.increment();
        assert!(val_c.is_one());
        val_c.decrement();
        assert!(val_c.is_zero());
        val_c.decrement();
        assert_eq!(max, val_c);
        val_c.decrement();
        assert_eq!(bfield_elem!(BFieldElement::MAX - 1), val_c);
    }

    proptest! {
        #[test]
        fn identity_tests(n in 0u128..BFieldElement::MAX) {
            let zero = bfield_elem!(0);
            let one = bfield_elem!(1);
            let other = bfield_elem!(n);

            prop_assert_eq!(other, zero + other, "left zero identity");
            prop_assert_eq!(other, other + zero, "right zero identity");
            prop_assert_eq!(other, one * other, "left one identity");
            prop_assert_eq!(other, other * one, "right one identity");
        }
    }

    #[test]
    fn inversion_test() {
        let one_inv = bfield_elem!(1);
        let two_inv = bfield_elem!(9223372034707292161);
        let three_inv = bfield_elem!(12297829379609722881);
        let four_inv = bfield_elem!(13835058052060938241);
        let five_inv = bfield_elem!(14757395255531667457);
        let six_inv = bfield_elem!(15372286724512153601);
        let seven_inv = bfield_elem!(2635249152773512046);
        let eight_inv = bfield_elem!(16140901060737761281);
        let nine_inv = bfield_elem!(4099276459869907627);
        let ten_inv = bfield_elem!(16602069662473125889);
        let eightfive_million_sixhundred_and_seventyone_onehundred_and_six_inv =
            bfield_elem!(13115294102219178839);
        assert_eq!(two_inv, bfield_elem!(2).inverse());
        assert_eq!(three_inv, bfield_elem!(3).inverse());
        assert_eq!(four_inv, bfield_elem!(4).inverse());
        assert_eq!(five_inv, bfield_elem!(5).inverse());
        assert_eq!(six_inv, bfield_elem!(6).inverse());
        assert_eq!(seven_inv, bfield_elem!(7).inverse());
        assert_eq!(eight_inv, bfield_elem!(8).inverse());
        assert_eq!(nine_inv, bfield_elem!(9).inverse());
        assert_eq!(ten_inv, bfield_elem!(10).inverse());
        assert_eq!(
            eightfive_million_sixhundred_and_seventyone_onehundred_and_six_inv,
            bfield_elem!(85671106).inverse()
        );

        let inverses = [
            one_inv,
            two_inv,
            three_inv,
            four_inv,
            five_inv,
            six_inv,
            seven_inv,
            eight_inv,
            nine_inv,
            ten_inv,
            eightfive_million_sixhundred_and_seventyone_onehundred_and_six_inv,
        ];
        let values = [
            bfield_elem!(1),
            bfield_elem!(2),
            bfield_elem!(3),
            bfield_elem!(4),
            bfield_elem!(5),
            bfield_elem!(6),
            bfield_elem!(7),
            bfield_elem!(8),
            bfield_elem!(9),
            bfield_elem!(10),
            bfield_elem!(85671106),
        ];
        let calculated_inverses = BFieldElement::batch_inversion(values.to_vec());
        assert_eq!(values.len(), calculated_inverses.len());
        let calculated_inverse_inverses =
            BFieldElement::batch_inversion(calculated_inverses.to_vec());
        for i in 0..calculated_inverses.len() {
            assert_eq!(inverses[i], calculated_inverses[i]);
            assert_eq!(calculated_inverse_inverses[i], values[i]);
        }

        let empty_inversion = BFieldElement::batch_inversion(vec![]);
        assert!(empty_inversion.is_empty());

        let singleton_inversion = BFieldElement::batch_inversion(vec![bfield_elem!(2)]);
        assert_eq!(1, singleton_inversion.len());
        assert_eq!(two_inv, singleton_inversion[0]);

        let duplet_inversion =
            BFieldElement::batch_inversion(vec![bfield_elem!(2), bfield_elem!(1)]);
        assert_eq!(2, duplet_inversion.len());
        assert_eq!(two_inv, duplet_inversion[0]);
        assert_eq!(one_inv, duplet_inversion[1]);
    }

    #[test]
    fn inversion_property_based_test() {
        let rands: Vec<i128> = generate_random_numbers(30, BFieldElement::MAX as i128);
        for rand in rands {
            assert!((bfield_elem!(rand as u128).inverse() * bfield_elem!(rand as u128)).is_one());
        }
    }

    #[test]
    fn batch_inversion_pbt() {
        let test_iterations = 100;
        for i in 0..test_iterations {
            let mut rng = rand::thread_rng();
            let rands: Vec<BFieldElement> = BFieldElement::random_elements(i, &mut rng);
            let rands_inv: Vec<BFieldElement> = BFieldElement::batch_inversion(rands.clone());
            assert_eq!(i as usize, rands_inv.len());
            for (mut rand, rand_inv) in izip!(rands, rands_inv) {
                assert!((rand * rand_inv).is_one());
                assert!((rand_inv * rand).is_one());
                assert_eq!(rand.inverse(), rand_inv);
                rand.increment();
                assert!(!(rand * rand_inv).is_one());
            }
        }
    }

    #[test]
    fn mul_div_plus_minus_property_based_test() {
        let rands: Vec<i128> = generate_random_numbers(300, BFieldElement::QUOTIENT as i128);
        for i in 1..rands.len() {
            let a = bfield_elem!(rands[i - 1] as u128);
            let b = bfield_elem!(rands[i] as u128);
            let ab = a * b;
            let a_o_b = a / b;
            let b_o_a = b / a;
            assert_eq!(a, ab / b);
            assert_eq!(b, ab / a);
            assert_eq!(a, a_o_b * b);
            assert_eq!(b, b_o_a * a);

            assert_eq!(a - b + b, a);
            assert_eq!(b - a + a, b);
            assert!((a - a).is_zero());
            assert!((b - b).is_zero());

            // Test the add/sub/mul assign operators
            let mut a_minus_b = a;
            a_minus_b -= b;
            assert_eq!(a - b, a_minus_b);

            let mut a_plus_b = a;
            a_plus_b += b;
            assert_eq!(a + b, a_plus_b);

            let mut a_mul_b = a;
            a_mul_b *= b;
            assert_eq!(a * b, a_mul_b);
        }
    }

    #[test]
    fn create_polynomial_test() {
        let a: Polynomial<BFieldElement> =
            Polynomial::new(vec![bfield_elem!(1), bfield_elem!(3), bfield_elem!(7)]);

        let b: Polynomial<BFieldElement> = Polynomial::new(vec![
            bfield_elem!(2),
            bfield_elem!(5),
            bfield_elem!(BFieldElement::MAX),
        ]);

        let expected: Polynomial<BFieldElement> =
            Polynomial::new(vec![bfield_elem!(3), bfield_elem!(8), bfield_elem!(6)]);

        assert_eq!(expected, a + b);
    }

    #[test]
    fn b_field_xgcd_test() {
        let a = 15;
        let b = 25;
        let expected_gcd_ab = 5;
        let (actual_gcd_ab, a_factor, b_factor) = other::xgcd(a, b);

        assert_eq!(expected_gcd_ab, actual_gcd_ab);
        assert_eq!(2, a_factor);
        assert_eq!(-1, b_factor);
        assert_eq!(expected_gcd_ab, a_factor * a + b_factor * b);
    }

    #[test]
    fn mod_pow_test() {
        // These values were found by finding primitive roots of unity and verifying
        // that they are group generators of the right order
        assert!(BFieldElement::new(281474976710656).mod_pow(4).is_one());
        assert_eq!(
            BFieldElement::new(281474976710656),
            BFieldElement::new(281474976710656).mod_pow(5)
        );
        assert!(BFieldElement::new(18446744069414584320).mod_pow(2).is_one()); // 18446744069414584320, 2nd
        assert!(BFieldElement::new(18446744069397807105).mod_pow(8).is_one());
        assert!(BFieldElement::new(2625919085333925275).mod_pow(10).is_one());
        assert!(BFieldElement::new(281474976645120).mod_pow(12).is_one());
    }

    #[test]
    fn get_primitive_root_of_unity_test() {
        for i in 1..33 {
            let power = 1 << i;
            let root_result = BFieldElement::ring_one().get_primitive_root_of_unity(power);
            match root_result.0 {
                Some(root) => println!("{} => {},", power, root),
                None => println!("Found no primitive root of unity for n = {}", power),
            };
            let root = root_result.0.unwrap();
            assert!(root.mod_pow(power as u64).is_one());
            assert!(!root.mod_pow(power as u64 / 2).is_one());
        }
    }
}