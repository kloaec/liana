//! Merging what a device sends back into the PSBT exported from Liana.
//!
//! Specter and Krux answer with a trimmed PSBT carrying only the signatures, so we merge them into
//! the original rather than handing the device's PSBT to Liana as-is.

use miniscript::bitcoin::Psbt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The device signed another transaction.
    OtherTransaction,
    /// The device sent back a PSBT without any new signature.
    NoNewSignature,
    /// The device finalized inputs: their signatures are in the witness, which Liana doesn't read.
    Finalized,
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::OtherTransaction => {
                f.write_str("This PSBT is for another transaction than the one you loaded.")
            }
            MergeError::Finalized => f.write_str(
                "The device finalized the transaction instead of adding its signatures. Liana \
                 needs the signatures: look for an option on the device to not finalize.",
            ),
            MergeError::NoNewSignature => f.write_str(
                "The device did not add any signature. Check the wallet is registered on it and \
                 that its key can sign this transaction.",
            ),
        }
    }
}

/// Copy the signatures of `signed` into `original`, returning how many were added.
pub fn merge_signatures(original: &mut Psbt, signed: &Psbt) -> Result<usize, MergeError> {
    if original.unsigned_tx.compute_txid() != signed.unsigned_tx.compute_txid() {
        return Err(MergeError::OtherTransaction);
    }
    let mut added = 0;
    for (input, signed_input) in original.inputs.iter_mut().zip(&signed.inputs) {
        for (pk, sig) in &signed_input.partial_sigs {
            if input.partial_sigs.insert(*pk, *sig).is_none() {
                added += 1;
            }
        }
        for (key, sig) in &signed_input.tap_script_sigs {
            if input.tap_script_sigs.insert(*key, *sig).is_none() {
                added += 1;
            }
        }
        if input.tap_key_sig.is_none() && signed_input.tap_key_sig.is_some() {
            input.tap_key_sig = signed_input.tap_key_sig;
            added += 1;
        }
    }
    if added == 0 {
        let finalized = signed
            .inputs
            .iter()
            .any(|i| i.final_script_witness.is_some() || i.final_script_sig.is_some());
        return Err(if finalized {
            MergeError::Finalized
        } else {
            MergeError::NoNewSignature
        });
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::tests_support::TEST_PSBT;
    use miniscript::bitcoin::{ecdsa, secp256k1, PublicKey};
    use std::str::FromStr;

    #[test]
    fn merges_trimmed_psbt() {
        let original = Psbt::from_str(TEST_PSBT).unwrap();
        // A trimmed answer: same transaction, only a signature in the input.
        let mut trimmed = Psbt::from_unsigned_tx(original.unsigned_tx.clone()).unwrap();
        let secp = secp256k1::Secp256k1::new();
        let sk = secp256k1::SecretKey::from_slice(&[1; 32]).unwrap();
        let pk = PublicKey::new(sk.public_key(&secp));
        let msg = secp256k1::Message::from_digest([2; 32]);
        let sig = ecdsa::Signature::sighash_all(secp.sign_ecdsa(&msg, &sk));
        trimmed.inputs[0].partial_sigs.insert(pk, sig);

        let mut merged = original.clone();
        assert_eq!(merge_signatures(&mut merged, &trimmed), Ok(1));
        // Nothing from the original is lost.
        assert_eq!(
            merged.inputs[0].witness_script,
            original.inputs[0].witness_script
        );
        assert_eq!(merged.inputs[0].partial_sigs.len(), 1);
        // Scanning the same answer again adds nothing.
        assert_eq!(
            merge_signatures(&mut merged, &trimmed),
            Err(MergeError::NoNewSignature)
        );
    }

    #[test]
    fn finalized_answer_is_explained() {
        let mut original = Psbt::from_str(TEST_PSBT).unwrap();
        let mut finalized = Psbt::from_unsigned_tx(original.unsigned_tx.clone()).unwrap();
        finalized.inputs[0].final_script_witness =
            Some(miniscript::bitcoin::Witness::from_slice(&[vec![1u8; 72]]));
        assert_eq!(
            merge_signatures(&mut original, &finalized),
            Err(MergeError::Finalized)
        );
    }

    #[test]
    fn rejects_other_transaction() {
        let mut original = Psbt::from_str(TEST_PSBT).unwrap();
        let mut other = original.clone();
        other.unsigned_tx.lock_time = miniscript::bitcoin::absolute::LockTime::from_consensus(1);
        assert_eq!(
            merge_signatures(&mut original, &other),
            Err(MergeError::OtherTransaction)
        );
    }
}
