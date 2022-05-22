use std::marker::PhantomData;
use std::{collections::HashMap, fmt::Debug};

use crate::util_types::mmr::shared::{leaf_index_to_peak_index, left_sibling, right_sibling};
use crate::{
    util_types::{
        mmr::shared::calculate_new_peaks_from_leaf_mutation,
        simple_hasher::{Hasher, ToDigest},
    },
    utils::has_unique_elements,
};

use super::{
    archival_mmr::ArchivalMmr,
    membership_proof::MembershipProof,
    mmr_trait::Mmr,
    shared::{
        bag_peaks, calculate_new_peaks_from_append, data_index_to_node_index, get_peak_height,
        get_peak_heights_and_peak_node_indices, leaf_count_to_node_count, parent,
        right_child_and_height,
    },
};

#[derive(Debug, Clone)]
pub struct MmrAccumulator<H: Hasher>
where
    H: Hasher,
{
    leaf_count: u128,
    peaks: Vec<H::Digest>,
    _hasher: PhantomData<H>,
}

impl<H> From<&ArchivalMmr<H>> for MmrAccumulator<H>
where
    H: Hasher,
    u128: ToDigest<H::Digest>,
{
    fn from(archive: &ArchivalMmr<H>) -> Self {
        Self {
            leaf_count: archive.count_leaves(),
            peaks: archive.get_peaks(),
            _hasher: PhantomData,
        }
    }
}

// u128: ToDigest<HashDigest>,
impl<H> MmrAccumulator<H>
where
    H: Hasher,
{
    pub fn init(peaks: Vec<H::Digest>, leaf_count: u128) -> Self {
        Self {
            leaf_count,
            peaks,
            _hasher: PhantomData,
        }
    }
}

impl<H> Mmr<H> for MmrAccumulator<H>
where
    H: Hasher,
    u128: ToDigest<H::Digest>,
{
    fn new(digests: Vec<H::Digest>) -> Self {
        // If all the hash digests already exist in memory, we might as well
        // build the shallow MMR from an archival MMR, since it doesn't give
        // asymptotically higher RAM consumption than building it without storing
        // all digests. At least, I think that's the case.
        // Clearly, this function could use less RAM if we don't build the entire
        // archival MMR.
        let leaf_count = digests.len() as u128;
        let archival = ArchivalMmr::<H>::new(digests);
        let peaks_and_heights = archival.get_peaks_with_heights();
        Self {
            _hasher: PhantomData,
            leaf_count,
            peaks: peaks_and_heights.iter().map(|x| x.0.clone()).collect(),
        }
    }

    fn bag_peaks(&self) -> H::Digest {
        bag_peaks::<H>(&self.peaks, leaf_count_to_node_count(self.leaf_count))
    }

    fn get_peaks(&self) -> Vec<H::Digest> {
        self.peaks.clone()
    }

    fn is_empty(&self) -> bool {
        self.leaf_count == 0
    }

    fn count_leaves(&self) -> u128 {
        self.leaf_count
    }

    fn append(&mut self, new_leaf: H::Digest) -> MembershipProof<H> {
        let (new_peaks, membership_proof) =
            calculate_new_peaks_from_append::<H>(self.leaf_count, self.peaks.clone(), new_leaf)
                .unwrap();
        self.peaks = new_peaks;
        self.leaf_count += 1;

        membership_proof
    }

    /// Mutate an existing leaf. It is the caller's responsibility that the
    /// membership proof is valid. If the membership proof is wrong, the MMR
    /// will end up in a broken state.
    fn mutate_leaf(&mut self, old_membership_proof: &MembershipProof<H>, new_leaf: &H::Digest) {
        let node_index = data_index_to_node_index(old_membership_proof.data_index);
        let hasher = H::new();
        let mut acc_hash: H::Digest = new_leaf.to_owned();
        let mut acc_index: u128 = node_index;
        for hash in old_membership_proof.authentication_path.iter() {
            let (acc_right, _acc_height) = right_child_and_height(acc_index);
            acc_hash = if acc_right {
                hasher.hash_pair(hash, &acc_hash)
            } else {
                hasher.hash_pair(&acc_hash, hash)
            };
            acc_index = parent(acc_index);
        }

        // This function is *not* secure when verified against *any* peak.
        // It **must** be compared against the correct peak.
        // Otherwise you could lie leaf_hash, data_index, authentication path
        let (peak_heights, _) = get_peak_heights_and_peak_node_indices(self.leaf_count);
        let expected_peak_height_res =
            get_peak_height(self.leaf_count, old_membership_proof.data_index);
        let expected_peak_height = match expected_peak_height_res {
            None => panic!("Did not find any peak height for (leaf_count, data_index) combination. Got: leaf_count = {}, data_index = {}", self.leaf_count, old_membership_proof.data_index),
            Some(eph) => eph,
        };

        let peak_height_index_res = peak_heights.iter().position(|x| *x == expected_peak_height);
        let peak_height_index = match peak_height_index_res {
            None => panic!("Did not find a matching peak"),
            Some(index) => index,
        };

        self.peaks[peak_height_index] = acc_hash;
    }

    /// Returns true of the `new_peaks` input matches the calculated new MMR peaks resulting from the
    /// provided appends and mutations.
    fn verify_batch_update(
        &self,
        new_peaks: &[H::Digest],
        appended_leafs: &[H::Digest],
        leaf_mutations: &[(H::Digest, MembershipProof<H>)],
    ) -> bool {
        // Verify that all leaf mutations operate on unique leafs and that they do
        // not exceed the total leaf count
        let manipulated_leaf_indices: Vec<u128> =
            leaf_mutations.iter().map(|x| x.1.data_index).collect();
        if !has_unique_elements(manipulated_leaf_indices.clone()) {
            return false;
        }

        // Disallow updating of out-of-bounds leafs
        if self.is_empty() && !manipulated_leaf_indices.is_empty()
            || !manipulated_leaf_indices.is_empty()
                && manipulated_leaf_indices.into_iter().max().unwrap() >= self.leaf_count
        {
            return false;
        }

        let mut leaf_mutation_target_values: Vec<H::Digest> =
            leaf_mutations.iter().map(|x| x.0.to_owned()).collect();
        let mut updated_membership_proofs: Vec<MembershipProof<H>> =
            leaf_mutations.iter().map(|x| x.1.to_owned()).collect();

        // Reverse the leaf mutation vectors, since I would like to apply them in the order
        // they were input to this function using `pop`.
        leaf_mutation_target_values.reverse();
        updated_membership_proofs.reverse();

        // First we apply all the leaf mutations
        let mut running_peaks: Vec<H::Digest> = self.peaks.clone();
        while let Some(membership_proof) = updated_membership_proofs.pop() {
            // `new_leaf_value` is guaranteed to exist since `leaf_mutation_target_values`
            // has the same length as `updated_membership_proofs`
            let new_leaf_value = leaf_mutation_target_values.pop().unwrap();

            // TODO: Should we verify the membership proof here?

            // Calculate the new peaks after mutating a leaf
            let running_peaks_res = calculate_new_peaks_from_leaf_mutation(
                &running_peaks,
                &new_leaf_value,
                self.leaf_count,
                &membership_proof,
            );
            running_peaks = match running_peaks_res {
                None => return false,
                Some(peaks) => peaks,
            };

            // Update all remaining membership proofs with this leaf mutation
            MembershipProof::<H>::batch_update_from_leaf_mutation(
                &mut updated_membership_proofs,
                &membership_proof,
                &new_leaf_value,
            );
        }

        // Then apply all the leaf appends
        let mut new_leafs_cloned: Vec<H::Digest> = appended_leafs.to_vec();

        // Reverse the new leafs to apply them in the same order as they were input,
        // using pop
        new_leafs_cloned.reverse();

        // Apply all leaf appends and
        let mut running_leaf_count = self.leaf_count;
        while let Some(new_leaf_for_append) = new_leafs_cloned.pop() {
            let append_res = calculate_new_peaks_from_append::<H>(
                running_leaf_count,
                running_peaks,
                new_leaf_for_append,
            );
            let (calculated_new_peaks, _new_membership_proof) = match append_res {
                None => return false,
                Some((peaks, mp)) => (peaks, mp),
            };
            running_peaks = calculated_new_peaks;
            running_leaf_count += 1;
        }

        running_peaks == new_peaks
    }

    fn batch_mutate_leaf_and_update_mps(
        &mut self,
        membership_proofs: &mut Vec<MembershipProof<H>>,
        mut mutation_data: Vec<(MembershipProof<H>, <H as Hasher>::Digest)>,
    ) -> Vec<u128> {
        // Calculate all derivable paths
        let mut new_ap_digests: HashMap<u128, H::Digest> = HashMap::new();
        let hasher = H::new();

        // Calculate the derivable digests from a number of leaf mutations and their
        // associated authentication paths. Notice that all authentication paths
        // are only valid *prior* to any updates. They get invalidated (unless updated)
        // throughout the updating as their neighbor leaf digests change values.
        // The hash map `new_ap_digests` takes care of that.
        while let Some((ap, new_leaf)) = mutation_data.pop() {
            let mut node_index = data_index_to_node_index(ap.data_index);
            let former_value = new_ap_digests.insert(node_index, new_leaf.clone());
            assert!(
                former_value.is_none(),
                "Duplicated leaf indices are not allowed in membership proof updater"
            );
            let mut acc_hash: H::Digest = new_leaf.to_owned();

            for (count, hash) in ap.authentication_path.iter().enumerate() {
                // If sibling node is something that has already been calculated, we use that
                // hash digest. Otherwise we use the one in our authentication path.
                let (right, height) = right_child_and_height(node_index);
                if right {
                    let left_sibling_index = left_sibling(node_index, height);
                    let sibling_hash: &H::Digest = match new_ap_digests.get(&left_sibling_index) {
                        Some(h) => h,
                        None => hash,
                    };
                    acc_hash = hasher.hash_pair(sibling_hash, &acc_hash);

                    // Find parent node index
                    node_index += 1;
                } else {
                    let right_sibling_index = right_sibling(node_index, height);
                    let sibling_hash: &H::Digest = match new_ap_digests.get(&right_sibling_index) {
                        Some(h) => h,
                        None => hash,
                    };
                    acc_hash = hasher.hash_pair(&acc_hash, sibling_hash);

                    // Find parent node index
                    node_index += 1 << (height + 1);
                }

                // The last hash calculated is the peak hash
                // This is not inserted in the hash map, as it will never be in any
                // authentication path
                if count < ap.authentication_path.len() - 1 {
                    new_ap_digests.insert(node_index, acc_hash.clone());
                }
            }

            // Update the peak
            let peaks_index = leaf_index_to_peak_index(ap.data_index, self.count_leaves());
            self.peaks[peaks_index as usize] = acc_hash;
        }

        // Update all the supplied membership proofs
        let mut modified_membership_proof_indices: Vec<u128> = vec![];
        for (i, membership_proof) in membership_proofs.iter_mut().enumerate() {
            let ap_indices = membership_proof.get_node_indices();

            // Some of the hashes in may `membership_proof` need to be updated. We can loop over
            // `authentication_path_indices` and check if the element is contained `deducible_hashes`.
            // If it is, then the appropriate element in `membership_proof.authentication_path` needs to
            // be replaced with an element from `deducible_hashes`.
            for (digest, authentication_path_indices) in membership_proof
                .authentication_path
                .iter_mut()
                .zip(ap_indices.into_iter())
            {
                // Any number of hashes can be updated in the authentication path, since
                // we're modifying multiple leaves in the MMR
                if new_ap_digests.contains_key(&authentication_path_indices) {
                    *digest = new_ap_digests[&authentication_path_indices].clone();
                    modified_membership_proof_indices.push(i as u128);
                }
            }
        }

        modified_membership_proof_indices.dedup();
        modified_membership_proof_indices
    }
}

#[cfg(test)]
mod accumulator_mmr_tests {
    use std::cmp;

    use itertools::izip;
    use rand::{thread_rng, Rng, RngCore};

    use crate::util_types::blake3_wrapper::Blake3Hash;
    use crate::utils::generate_random_numbers_u128;

    use super::*;

    #[test]
    fn conversion_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let leaf_hashes: Vec<Digest> = vec![14u128, 15u128, 16u128]
            .into_iter()
            .map(|x| x.into())
            .collect();

        let archival_mmr: ArchivalMmr<Hasher> = ArchivalMmr::<Hasher>::new(leaf_hashes.clone());
        let accumulator_mmr: MmrAccumulator<Hasher> = (&archival_mmr).into();

        assert_eq!(archival_mmr.get_peaks(), accumulator_mmr.get_peaks());
        assert_eq!(archival_mmr.bag_peaks(), accumulator_mmr.bag_peaks());
        assert_eq!(archival_mmr.is_empty(), accumulator_mmr.is_empty());
        assert!(!archival_mmr.is_empty());
        assert_eq!(archival_mmr.count_leaves(), accumulator_mmr.count_leaves());
        assert_eq!(3, accumulator_mmr.count_leaves());
    }

    #[test]
    fn verify_batch_update_single_append_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let leaf_hashes_start: Vec<Digest> = vec![14u128, 15u128, 16u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let appended_leaf: Digest = 17u128.into();

        let leaf_hashes_end: Vec<Digest> = vec![14u128, 15u128, 16u128, 17u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let accumulator_mmr_start: MmrAccumulator<Hasher> =
            MmrAccumulator::<Hasher>::new(leaf_hashes_start.clone());
        let accumulator_mmr_end: MmrAccumulator<Hasher> =
            MmrAccumulator::new(leaf_hashes_end.clone());
        assert!(accumulator_mmr_start.verify_batch_update(
            &accumulator_mmr_end.get_peaks(),
            &[appended_leaf],
            &[]
        ));
    }

    #[test]
    fn verify_batch_update_single_mutate_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let leaf_hashes_start: Vec<Digest> = vec![14u128, 15u128, 16u128, 18u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let new_leaf_value: Digest = 17u128.into();
        let leaf_hashes_end: Vec<Digest> = vec![14u128, 15u128, 16u128, 17u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let accumulator_mmr_start: MmrAccumulator<Hasher> =
            MmrAccumulator::<Hasher>::new(leaf_hashes_start.clone());
        let archive_mmr_start = ArchivalMmr::new(leaf_hashes_start);
        let membership_proof = archive_mmr_start.prove_membership(3).0;
        let accumulator_mmr_end: MmrAccumulator<Hasher> =
            MmrAccumulator::new(leaf_hashes_end.clone());
        assert!(accumulator_mmr_start.verify_batch_update(
            &accumulator_mmr_end.get_peaks(),
            &[],
            &[(new_leaf_value.clone(), membership_proof.clone())]
        ));

        // Verify that repeated indices are disallowed
        assert!(!accumulator_mmr_start.verify_batch_update(
            &accumulator_mmr_end.get_peaks(),
            &[],
            &[
                (new_leaf_value.clone(), membership_proof.clone()),
                (new_leaf_value.clone(), membership_proof.clone())
            ]
        ));
    }

    #[test]
    fn verify_batch_update_two_append_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let leaf_hashes_start: Vec<Digest> = vec![14u128, 15u128, 16u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let appended_leafs: Vec<Digest> =
            vec![25u128, 29u128].into_iter().map(|x| x.into()).collect();
        let leaf_hashes_end: Vec<Digest> = vec![14u128, 15u128, 16u128, 25u128, 29u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let accumulator_mmr_start: MmrAccumulator<Hasher> =
            MmrAccumulator::new(leaf_hashes_start.clone());
        let accumulator_mmr_end: MmrAccumulator<Hasher> =
            MmrAccumulator::new(leaf_hashes_end.clone());
        assert!(accumulator_mmr_start.verify_batch_update(
            &accumulator_mmr_end.get_peaks(),
            &appended_leafs,
            &[]
        ));
    }

    #[test]
    fn verify_batch_update_two_mutate_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let leaf_hashes_start: Vec<Digest> = vec![14u128, 15u128, 16u128, 17u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let new_leafs: Vec<Digest> = vec![20u128, 21u128].into_iter().map(|x| x.into()).collect();
        let leaf_hashes_end: Vec<Digest> = vec![14u128, 20u128, 16u128, 21u128]
            .into_iter()
            .map(|x| x.into())
            .collect();
        let accumulator_mmr_start: MmrAccumulator<Hasher> =
            MmrAccumulator::<Hasher>::new(leaf_hashes_start.clone());
        let archive_mmr_start = ArchivalMmr::new(leaf_hashes_start);
        let membership_proof1 = archive_mmr_start.prove_membership(1).0;
        let membership_proof3 = archive_mmr_start.prove_membership(3).0;
        let accumulator_mmr_end: MmrAccumulator<Hasher> =
            MmrAccumulator::new(leaf_hashes_end.clone());
        assert!(accumulator_mmr_start.verify_batch_update(
            &accumulator_mmr_end.get_peaks(),
            &[],
            &[
                (new_leafs[0], membership_proof1),
                (new_leafs[1], membership_proof3)
            ]
        ));
    }

    #[test]
    fn batch_mutate_leaf_and_update_mps_test() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        let mut prng = thread_rng();
        for mmr_leaf_count in 1..100 {
            let initial_leaf_digests: Vec<Digest> = (4000u128..4000u128 + mmr_leaf_count)
                .map(|x| x.into())
                .collect();
            let mut mmra: MmrAccumulator<Hasher> =
                MmrAccumulator::new(initial_leaf_digests.clone());
            let mut ammr: ArchivalMmr<Hasher> = ArchivalMmr::new(initial_leaf_digests.clone());

            let mutated_leaf_count = prng.gen_range(0..mmr_leaf_count);
            let all_indices: Vec<u128> = (0..mmr_leaf_count).collect();

            // Pick indices for leaves that are being mutated
            let mut all_indices_mut0 = all_indices.clone();
            let mut mutated_leaf_indices: Vec<u128> = vec![];
            for _ in 0..mutated_leaf_count {
                mutated_leaf_indices.push(
                    all_indices_mut0.remove(prng.next_u32() as usize % all_indices_mut0.len()),
                );
            }

            // Pick membership proofs that we want to update
            let membership_proof_count = prng.gen_range(0..mmr_leaf_count);
            let mut all_indices_mut1 = all_indices.clone();
            let mut membership_proof_indices: Vec<u128> = vec![];
            for _ in 0..membership_proof_count {
                membership_proof_indices.push(
                    all_indices_mut1.remove(prng.next_u32() as usize % all_indices_mut1.len()),
                );
            }

            // Calculate the terminal leafs, as they look after the batch leaf mutation
            // that we are preparing to execute
            let new_leafs: Vec<Digest> =
                (6u128..6 + mutated_leaf_count).map(|x| x.into()).collect();
            let mut terminal_leafs: Vec<Digest> = initial_leaf_digests;
            for (i, new_leaf) in mutated_leaf_indices.iter().zip(new_leafs.iter()) {
                terminal_leafs[*i as usize] = new_leaf.to_owned();
            }

            // Calculate the leafs digests associated with the membership proofs, as they look
            // *after* the batch leaf mutation
            let mut terminal_leafs_for_mps: Vec<Digest> = vec![];
            for i in membership_proof_indices.iter() {
                terminal_leafs_for_mps.push(terminal_leafs[*i as usize]);
            }

            // Construct the mutation data
            let mutated_leaf_mps: Vec<MembershipProof<Hasher>> = mutated_leaf_indices
                .iter()
                .map(|i| ammr.prove_membership(*i).0)
                .collect();
            let mutation_data: Vec<(MembershipProof<Hasher>, Digest)> = mutated_leaf_mps
                .into_iter()
                .zip(new_leafs.into_iter())
                .collect();

            assert_eq!(mutated_leaf_count as usize, mutation_data.len());

            let original_membership_proofs: Vec<MembershipProof<Hasher>> = membership_proof_indices
                .iter()
                .map(|i| ammr.prove_membership(*i).0)
                .collect();

            // Do the update on both MMRs
            let mut mmra_mps = original_membership_proofs.clone();
            let mut ammr_mps = original_membership_proofs.clone();
            let mut ammr_copy = ammr.clone();
            mmra.batch_mutate_leaf_and_update_mps(&mut mmra_mps, mutation_data.clone());
            ammr.batch_mutate_leaf_and_update_mps(&mut ammr_mps, mutation_data.clone());

            // Verify that both MMRs end up with same peaks
            assert_eq!(mmra.get_peaks(), ammr.get_peaks());

            // Verify that membership proofs from AMMR and MMRA are equal
            assert_eq!(membership_proof_count as usize, mmra_mps.len());
            assert_eq!(membership_proof_count as usize, ammr_mps.len());
            assert_eq!(ammr_mps, mmra_mps);

            // Verify that all membership proofs still work
            assert!(mmra_mps
                .iter()
                .zip(terminal_leafs_for_mps.iter())
                .all(|(mp, leaf)| mp.verify(&mmra.get_peaks(), &leaf, mmra.count_leaves()).0));

            // Manually construct an MMRA from the new data and verify that peaks and leaf count matches
            assert!(
                mutated_leaf_count == 0 || ammr_copy.get_peaks() != ammr.get_peaks(),
                "If mutated leaf count is non-zero, at least on peaks must be different"
            );
            mutation_data.into_iter().for_each(|(mp, digest)| {
                ammr_copy.mutate_leaf_raw(mp.data_index, digest);
            });
            assert_eq!(ammr_copy.get_peaks(), ammr.get_peaks(), "Mutation though batch mutation function must transform the MMR like a list of individual leaf mutations");
        }
    }

    #[test]
    fn verify_batch_update_pbt() {
        type Digest = Blake3Hash;
        type Hasher = blake3::Hasher;

        for start_size in 1..35 {
            let leaf_hashes_start: Vec<Digest> = (4000u128..4000u128 + start_size)
                .map(|x| x.into())
                .collect();
            let bad_digests: Vec<Digest> =
                (12u128..12u128 + start_size).map(|x| x.into()).collect();
            let bad_mmr = ArchivalMmr::<Hasher>::new(bad_digests.clone());
            let bad_membership_proof: MembershipProof<Hasher> = bad_mmr.prove_membership(0).0;
            let bad_membership_proof_digest = bad_digests[0];
            let bad_leaf: Digest = 8765432165123u128.into();
            let archival_mmr_init = ArchivalMmr::<Hasher>::new(leaf_hashes_start.clone());
            let accumulator_mmr = MmrAccumulator::<Hasher>::new(leaf_hashes_start.clone());
            for append_size in 0..18 {
                let appends: Vec<Digest> = (2000u128..2000u128 + append_size)
                    .map(|x| x.into())
                    .collect();
                let mutate_count = cmp::min(12, start_size);
                for mutate_size in 0..mutate_count {
                    let new_leaf_values: Vec<Digest> =
                        (13u128..13u128 + mutate_size).map(|x| x.into()).collect();
                    let mut mutated_indices =
                        generate_random_numbers_u128(mutate_size as usize, Some(start_size));

                    // Ensure that indices are unique since batch updating cannot update
                    // the same leaf twice in one go
                    mutated_indices.sort();
                    mutated_indices.dedup();

                    // Create the expected MMRs
                    let mut leaf_hashes_mutated = leaf_hashes_start.clone();
                    for (index, new_leaf) in izip!(mutated_indices.clone(), new_leaf_values.clone())
                    {
                        leaf_hashes_mutated[index as usize] = new_leaf;
                    }
                    for appended_digest in appends.iter() {
                        leaf_hashes_mutated.push(appended_digest.to_owned());
                    }

                    let mutated_archival_mmr =
                        ArchivalMmr::<Hasher>::new(leaf_hashes_mutated.clone());
                    let mutated_accumulator_mmr = ArchivalMmr::<Hasher>::new(leaf_hashes_mutated);
                    let expected_new_peaks_from_archival = mutated_archival_mmr.get_peaks();
                    let expected_new_peaks_from_accumulator = mutated_accumulator_mmr.get_peaks();
                    assert_eq!(
                        expected_new_peaks_from_archival,
                        expected_new_peaks_from_accumulator
                    );

                    // Create the inputs to the method call
                    let membership_proofs: Vec<MembershipProof<Hasher>> = mutated_indices
                        .iter()
                        .map(|&i| archival_mmr_init.prove_membership(i).0)
                        .collect();
                    let mut leaf_mutations: Vec<(Digest, MembershipProof<Hasher>)> =
                        new_leaf_values
                            .clone()
                            .into_iter()
                            .zip(membership_proofs.into_iter())
                            .map(|(v, mp)| (v, mp))
                            .collect();
                    assert!(accumulator_mmr.verify_batch_update(
                        &expected_new_peaks_from_accumulator,
                        &appends,
                        &leaf_mutations
                    ));
                    assert!(archival_mmr_init.verify_batch_update(
                        &expected_new_peaks_from_accumulator,
                        &appends,
                        &leaf_mutations
                    ));

                    // Negative tests
                    let mut bad_appends = appends.clone();
                    if append_size > 0 && mutate_size > 0 {
                        // bad append vector
                        bad_appends[(mutated_indices[0] % append_size) as usize] = bad_leaf;
                        assert!(!accumulator_mmr.verify_batch_update(
                            &expected_new_peaks_from_accumulator,
                            &bad_appends,
                            &leaf_mutations
                        ));

                        // Bad membership proof
                        leaf_mutations[mutated_indices[0] as usize % mutated_indices.len()].0 =
                            bad_membership_proof_digest.clone();
                        assert!(!accumulator_mmr.verify_batch_update(
                            &expected_new_peaks_from_accumulator,
                            &appends,
                            &leaf_mutations
                        ));
                        leaf_mutations[mutated_indices[0] as usize % mutated_indices.len()].1 =
                            bad_membership_proof.clone();
                        assert!(!accumulator_mmr.verify_batch_update(
                            &expected_new_peaks_from_accumulator,
                            &appends,
                            &leaf_mutations
                        ));
                    }
                }
            }
        }
    }
}
