use crate::shared_math::b_field_element::BFieldElement;
use crate::shared_math::stark::BoundaryConstraint;
use crate::shared_math::traits::IdentityValues;

use super::mpolynomial::MPolynomial;
use super::polynomial::Polynomial;
use super::traits::{GetGeneratorDomain, ModPowU64};

// TODO: Make this work for XFieldElement via trait.
#[derive(Debug, Clone)]
pub struct RescuePrime {
    pub m: usize,
    // rate: usize,
    // capacity: usize,
    pub steps_count: usize,
    pub alpha: u64,
    pub alpha_inv: u64,
    pub mds: Vec<Vec<BFieldElement>>,
    pub mds_inv: Vec<Vec<BFieldElement>>,
    pub round_constants: Vec<BFieldElement>,
}

impl RescuePrime {
    fn hash_round(
        &self,
        input_state: Vec<BFieldElement>,
        round_number: usize,
    ) -> Vec<BFieldElement> {
        // S-box
        let mut state: Vec<BFieldElement> = input_state
            .iter()
            .map(|&v| v.mod_pow_u64(self.alpha))
            .collect();

        // Matrix
        let mut temp: Vec<BFieldElement> = vec![input_state[0].ring_zero(); self.m];
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.m {
            for j in 0..self.m {
                temp[i] = temp[i].clone() + self.mds[i][j].clone() * state[j].clone();
            }
        }

        // Add rounding constants
        state = temp
            .into_iter()
            .enumerate()
            .map(|(i, val)| val + self.round_constants[2 * round_number * self.m + i].clone())
            .collect();

        // Backward half-round
        // S-box
        state = state
            .iter()
            .map(|v| v.mod_pow(self.alpha_inv.clone()))
            .collect();

        // Matrix
        temp = vec![input_state[0].ring_zero(); self.m];
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.m {
            for j in 0..self.m {
                temp[i] = temp[i].clone() + self.mds[i][j].clone() * state[j].clone();
            }
        }

        // Add rounding constants
        state = temp
            .into_iter()
            .enumerate()
            .map(|(i, val)| {
                val + self.round_constants[2 * round_number * self.m + self.m + i].clone()
            })
            .collect();

        state
    }

    /// Return the Rescue-Prime hash value
    pub fn hash(&self, input: &BFieldElement) -> BFieldElement {
        let mut state = vec![input.ring_zero(); self.m];
        state[0] = input.to_owned();

        state = (0..self.steps_count).fold(state, |state, i| self.hash_round(state, i));

        state[0].clone()
    }

    pub fn trace(&self, input: &BFieldElement) -> Vec<Vec<BFieldElement>> {
        let mut trace: Vec<Vec<BFieldElement>> = vec![];
        let mut state = vec![input.ring_zero(); self.m];
        state[0] = input.to_owned();
        trace.push(state.clone());

        // It could be cool to write this with `scan` instead of a for-loop, but I couldn't get that to work
        for i in 0..self.steps_count {
            let next_state = self.hash_round(state, i);
            trace.push(next_state.clone());
            state = next_state;
        }

        trace
    }

    pub fn eval_and_trace(
        &self,
        input: &BFieldElement,
    ) -> (BFieldElement, Vec<Vec<BFieldElement>>) {
        let trace = self.trace(input);
        let output = trace.last().unwrap()[0].clone();

        (output, trace)
    }

    /// Return a pair of a list of polynomials, first element in the pair,
    /// (first_round_constants[register], second_round_constants[register])
    pub fn get_round_constant_polynomials(
        &self,
        omicron: BFieldElement,
    ) -> (
        Vec<MPolynomial<BFieldElement>>,
        Vec<MPolynomial<BFieldElement>>,
    ) {
        let domain = omicron.get_generator_domain();
        let mut first_round_constants: Vec<MPolynomial<BFieldElement>> = vec![];
        for i in 0..self.m {
            let values: Vec<BFieldElement> = self
                .round_constants
                .clone()
                .into_iter()
                .skip(i)
                .step_by(2 * self.m)
                .collect();
            // let coefficients = intt(&values, omicron);
            let points: Vec<(BFieldElement, BFieldElement)> = domain
                .clone()
                .iter()
                .zip(values.iter())
                .map(|(x, y)| (x.to_owned(), y.to_owned()))
                .collect();
            let coefficients = Polynomial::slow_lagrange_interpolation(&points).coefficients;
            first_round_constants.push(MPolynomial::lift(Polynomial { coefficients }, 0));
        }

        let mut second_round_constants: Vec<MPolynomial<BFieldElement>> = vec![];
        for i in 0..self.m {
            let values: Vec<BFieldElement> = self
                .round_constants
                .clone()
                .into_iter()
                .skip(i + self.m)
                .step_by(2 * self.m)
                .collect();
            // let coefficients = intt(&values, omicron);
            let points: Vec<(BFieldElement, BFieldElement)> = domain
                .clone()
                .iter()
                .zip(values.iter())
                .map(|(x, y)| (x.to_owned(), y.to_owned()))
                .collect();
            let coefficients = Polynomial::slow_lagrange_interpolation(&points).coefficients;
            second_round_constants.push(MPolynomial::lift(Polynomial { coefficients }, 0));
        }

        (first_round_constants, second_round_constants)
    }

    // Returns the multivariate polynomial which takes the triplet (domain, trace, next_trace) and
    // returns composition polynomial, which is the evaluation of the air for a specific trace.
    // AIR: [F_p x F_p^m x F_p^m] --> F_p^m
    // The composition polynomial values are low-degree polynomial combinations
    // (as opposed to linear combinations) of the values:
    // `domain` (scalar), `trace` (vector), `next_trace` (vector).
    pub fn get_air_constraints(&self, omicron: BFieldElement) -> Vec<MPolynomial<BFieldElement>> {
        let (first_step_constants, second_step_constants) =
            self.get_round_constant_polynomials(omicron);

        let variables = MPolynomial::variables(1 + 2 * self.m, omicron.ring_one());
        let previous_state = &variables[1..(self.m + 1)];
        let next_state = &variables[(self.m + 1)..(2 * self.m + 1)];
        let one = omicron.ring_one();
        let mut air: Vec<MPolynomial<BFieldElement>> = vec![];

        // TODO: Consider refactoring MPolynomial<BFieldElement>
        // ::mod_pow(exp: BigInt, one: BFieldElement) into
        // ::mod_pow_u64(exp: u64)
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.m {
            let mut lhs = MPolynomial::from_constant(omicron.ring_zero());
            for k in 0..self.m {
                lhs = lhs
                    + previous_state[k]
                        .mod_pow(self.alpha.clone().into(), one.clone())
                        .scalar_mul(self.mds[i][k].clone());
            }
            lhs = lhs + first_step_constants[i].clone();

            let mut rhs = MPolynomial::from_constant(omicron.ring_zero());
            for k in 0..self.m {
                rhs = rhs
                    + (next_state[k].clone() - second_step_constants[k].clone())
                        .scalar_mul(self.mds_inv[i][k].clone());
            }
            rhs = rhs.mod_pow(self.alpha.clone().into(), one.clone());

            air.push(lhs - rhs);
        }

        air
    }

    pub fn get_boundary_constraints(
        &self,
        output_element: BFieldElement,
    ) -> Vec<BoundaryConstraint> {
        vec![
            BoundaryConstraint {
                cycle: 0,
                register: 1,
                value: output_element.ring_zero(),
            },
            BoundaryConstraint {
                cycle: self.steps_count,
                register: 0,
                value: output_element.to_owned(),
            },
        ]
    }
}

#[cfg(test)]
mod rescue_prime_test {
    use super::*;
    use crate::shared_math::rescue_prime_params::rescue_prime_params_bfield_0;

    #[test]
    fn hash_test() {
        let rp = rescue_prime_params_bfield_0();

        // Calculated with stark-anatomy tutorial implementation, starting with hash(1)
        let one = BFieldElement::new(1);
        let expected_sequence: Vec<BFieldElement> = vec![
            16408223883448864076,
            14851226605068667585,
            2638999062907144857,
            11729682885064735215,
            18241842748565968364,
            12761136320817622587,
            6569784252060404379,
            7456670293305349839,
            12092401435052133560,
        ]
        .iter()
        .map(|elem| BFieldElement::new(*elem))
        .collect();

        let mut actual = rp.hash(&one);
        for expected in expected_sequence {
            assert_eq!(expected, actual);
            actual = rp.hash(&expected);
        }
    }

    #[test]
    fn air_is_zero_on_execution_trace_test() {
        let rp = rescue_prime_params_bfield_0();

        // rescue prime test vector 1
        let omicron_res = BFieldElement::get_primitive_root_of_unity(1 << 5);
        let omicron = omicron_res.0.unwrap();

        // Verify that the round constants polynomials are correct
        let (fst_rc_pol, snd_rc_pol) = rp.get_round_constant_polynomials(omicron);
        for step in 0..rp.steps_count {
            let point = vec![omicron.mod_pow(step as u64)];
            for register in 0..rp.m {
                let fst_eval = fst_rc_pol[register].evaluate(&point);
                assert_eq!(rp.round_constants[2 * step * rp.m + register], fst_eval);
            }
            for register in 0..rp.m {
                let snd_eval = snd_rc_pol[register].evaluate(&point);
                assert_eq!(
                    rp.round_constants[2 * step * rp.m + rp.m + register],
                    snd_eval
                );
            }
        }

        // There are 256 round constants, which is enough for 8 rounds (steps_count).
        // But we only run with 7 rounds (steps_count), so we add 1 to count right.
        let actual_round_constants = rp.round_constants.len();
        let expected_round_constants = (rp.steps_count + 1) * 2 * rp.m;
        assert_eq!(expected_round_constants, actual_round_constants);

        // Verify that the AIR constraints evaluation over the trace is zero along the trace
        println!("zomg!");
        let input_2 = BFieldElement::new(42);
        let trace = rp.trace(&input_2);
        println!("zomg!");
        let air_constraints = rp.get_air_constraints(omicron);
        println!("zomg!");

        for step in 0..rp.steps_count - 1 {
            println!("Step {}", step);
            for air_constraint in air_constraints.iter() {
                let mut point = vec![];
                point.push(omicron.mod_pow(step as u64));
                for i in 0..rp.m {
                    point.push(trace[step][i].clone());
                    point.push(trace[step + 1][i].clone());
                }
                // point.push(trace[step][1].clone());
                // point.push(trace[step + 1][1].clone());
                let eval = air_constraint.evaluate(&point);
                assert!(eval.is_zero());
            }
        }
    }
}
