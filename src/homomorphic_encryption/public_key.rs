use super::ciphertext::Ciphertext;
use super::polynomial::Polynomial;
use std::fmt;

// Public key should maybe also have a `size` field
#[derive(Debug)]
pub struct PublicKey<'a> {
    pub a: Polynomial<'a>,
    pub b: Polynomial<'a>,
}

impl fmt::Display for PublicKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(b={}, a={})", self.b, self.a)
    }
}

impl<'a> PublicKey<'a> {
    pub fn encrypt(&self, plain_text_modulus: i128, pt: i128) -> Ciphertext<'a> {
        let e1 = Polynomial::gen_normal_poly(self.a.pqr);
        // println!("e1: {}", e1);
        let e2 = Polynomial::gen_normal_poly(self.a.pqr);
        // println!("e2: {}", e2);
        let delta = self.a.pqr.q / plain_text_modulus;
        let m = Polynomial::polynomium_from_int(pt, self.a.pqr).scalar_modulus(plain_text_modulus);
        // println!("m: {}", m);
        let scaled_m = m.scalar_mul(delta).scalar_modulus(self.a.pqr.q);
        // println!("scaled_m: {}", scaled_m);
        let u = Polynomial::gen_binary_poly(self.a.pqr)
            .modulus()
            .normalize();
        // println!("u: {}", u);
        let ct0 = self.b.mul(&u).add(&e1).add(&scaled_m).modulus().normalize();
        let ct1 = self.a.mul(&u).add(&e2).modulus().normalize();
        Ciphertext { ct0, ct1 }
    }
}
