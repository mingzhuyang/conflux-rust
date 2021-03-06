// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

const DEFAULT_NUMBER_OF_KEYS: usize = 100000;

fn generate_keys(number_of_keys: usize) -> Vec<[u8; 4]> {
    let mut rng = get_rng_for_test();

    let mut keys_num: Vec<u32> = Default::default();

    for i in 0..number_of_keys {
        keys_num.push(rng.gen());
    }

    keys_num.sort();

    let mut keys: Vec<[u8; 4]> = Default::default();
    let mut last_key = keys_num[0];
    for key in &keys_num[1..number_of_keys] {
        if *key != last_key {
            keys.push(unsafe { mem::transmute::<u32, [u8; 4]>(key.clone()) });
        }
        last_key = *key;
    }

    rng.shuffle(&mut keys);
    keys
}

fn get_rng_for_test() -> ChaChaRng { ChaChaRng::from_seed([123; 32]) }

#[test]
fn test_set_get() {
    let mut rng = get_rng_for_test();
    let state_manager = new_state_manager_for_testing();
    let mut state = state_manager.get_state_at(H256::default()).unwrap();
    let mut keys: Vec<[u8; 4]> = generate_keys(DEFAULT_NUMBER_OF_KEYS)
        .iter()
        .filter(|_| rng.gen_bool(0.5))
        .cloned()
        .collect();

    println!("Testing with {} set operations.", keys.len());

    for key in &keys {
        state.set(key, key).expect("Failed to insert key.");
    }

    rng.shuffle(keys.as_mut());

    for key in &keys {
        let value = state
            .get(key)
            .expect("Failed to get key.")
            .expect("Failed to get key");
        let equal = key.eq(value.as_ref());
        assert_eq!(equal, true);
    }

    let mut epoch_id = H256::default();
    epoch_id[0] = 1;
    state.compute_state_root().unwrap();
    state.commit(epoch_id).unwrap();
}

#[test]
fn test_get_set_at_second_commit() {
    let rng = get_rng_for_test();
    let state_manager = new_state_manager_for_testing();
    let keys: Vec<[u8; 4]> = generate_keys(DEFAULT_NUMBER_OF_KEYS);
    let set_size = 10000;
    let (keys_0, keys_1_new, keys_remain, keys_1_overwritten) = (
        &keys[0..set_size * 2],
        &keys[set_size * 2..set_size * 3],
        &keys[0..set_size],
        &keys[set_size..set_size * 2],
    );

    let parent_epoch_0 = H256::default();
    let mut state_0 = state_manager.get_state_at(parent_epoch_0).unwrap();
    println!("Setting state_0 with {} keys.", keys_0.len());

    for key in keys_0 {
        state_0.set(key, key).expect("Failed to insert key.");
    }

    let mut epoch_id_0 = H256::default();;
    epoch_id_0[0] = 1;
    state_0.compute_state_root().unwrap();
    state_0.commit(epoch_id_0).unwrap();

    let mut state_1 = state_manager.get_state_at(epoch_id_0).unwrap();
    println!("Set new {} keys for state_1.", keys_1_new.len(),);
    for key in keys_1_new {
        let value = vec![&key[..], &key[..]].concat();
        state_1
            .set(key, value.as_slice())
            .expect("Failed to insert key.");
    }

    println!(
        "Reading overlapping {} keys from state_0 and set new keys for state_1.",
        keys_1_overwritten.len(),
    );
    for key in keys_1_overwritten {
        let old_value = state_1
            .get(key)
            .expect("Failed to get key.")
            .expect("Failed to get key");
        let equal = key.eq(old_value.as_ref());
        assert_eq!(equal, true);
        let value = vec![&key[..], &key[..]].concat();
        state_1
            .set(key, value.as_slice())
            .expect("Failed to insert key.");
    }

    println!(
        "Reading untouched {} keys from state_0 in state_1.",
        keys_remain.len(),
    );
    for key in keys_remain {
        let value = state_1
            .get(key)
            .expect("Failed to get key.")
            .expect("Failed to get key");
        let equal = key.eq(value.as_ref());
        assert_eq!(equal, true);
    }

    println!(
        "Reading modified {} keys in state_1.",
        keys_1_overwritten.len(),
    );
    for key in keys_1_overwritten {
        let value = state_1
            .get(key)
            .expect("Failed to get key.")
            .expect("Failed to get key");
        let expected_value = vec![&key[..], &key[..]].concat();
        let equal = expected_value.eq(&value.as_ref());
        assert_eq!(equal, true);
    }

    let mut epoch_id_1 = H256::default();;
    epoch_id_1[0] = 2;
    state_1.compute_state_root().unwrap();
    state_1.commit(epoch_id_1).unwrap();
}

#[test]
fn test_set_delete() {
    let mut rng = get_rng_for_test();
    let state_manager = new_state_manager_for_testing();

    let mut state = state_manager.get_state_at(H256::default()).unwrap();
    let empty_state_root = state.compute_state_root().unwrap();

    let mut keys: Vec<[u8; 4]> = generate_keys(DEFAULT_NUMBER_OF_KEYS);
    let (keys_0, keys_1) = (
        &keys[0..DEFAULT_NUMBER_OF_KEYS / 2],
        &keys[DEFAULT_NUMBER_OF_KEYS / 2..],
    );

    println!("Testing with {} set operations.", keys.len());

    // Insert part 1 and commit.
    for key in keys_0.iter() {
        state.set(key, key).expect("Failed to insert key.");
    }
    let mut epoch_id = H256::default();
    epoch_id[0] = 1;
    state.compute_state_root().unwrap();
    state.commit(epoch_id).unwrap();

    // In second state, insert part 2, then delete everything.
    let mut state = state_manager.get_state_at(epoch_id).unwrap();
    for key in keys_1.iter() {
        state.set(key, key).expect("Failed to insert key.");
    }

    rng.shuffle(keys.as_mut());

    println!("Testing with {} delete operations.", keys.len());
    let mut count = 0;
    for key in &keys {
        count += 1;
        let value = state
            .delete(key)
            .expect("Failed to delete key.")
            .expect("Failed to get key");
        let equal = key.eq(value.as_ref());
        assert_eq!(equal, true);
    }

    let mut epoch_id = H256::default();
    epoch_id[0] = 2;
    let state_root = state.compute_state_root().unwrap();
    state.commit(epoch_id).unwrap();

    assert_eq!(state_root, empty_state_root);
}

#[test]
fn test_set_delete_all() {
    let mut rng = get_rng_for_test();
    let state_manager = new_state_manager_for_testing();

    let mut state = state_manager.get_state_at(H256::default()).unwrap();
    let empty_state_root = state.compute_state_root().unwrap();

    let mut keys: Vec<[u8; 4]> = generate_keys(DEFAULT_NUMBER_OF_KEYS);
    let (keys_0, keys_1) = (
        &keys[0..DEFAULT_NUMBER_OF_KEYS / 2],
        &keys[DEFAULT_NUMBER_OF_KEYS / 2..],
    );

    println!("Testing with {} set operations.", keys.len());

    // Insert part 1 and commit.
    for key in keys_0.iter() {
        state
            .set(vec![&key[..], &key[..]].concat().as_slice(), key)
            .expect("Failed to insert key.");
    }
    let mut epoch_id = H256::default();
    epoch_id[0] = 1;
    state.compute_state_root().unwrap();
    state.commit(epoch_id).unwrap();

    // In second state, insert part 2, then delete everything.
    let mut state = state_manager.get_state_at(epoch_id).unwrap();
    for key in keys_1.iter() {
        state
            .set(vec![&key[..], &key[..]].concat().as_slice(), key)
            .expect("Failed to insert key.");
    }

    rng.shuffle(keys.as_mut());

    println!("Testing with {} delete_all operations.", keys.len());
    let mut values = Vec::with_capacity(keys.len());
    let mut count = 0;
    for key in &keys {
        count += 1;

        let key_prefix = &key[0..(2 + rng.gen::<usize>() % 2)];

        let value =
            state.delete_all(key_prefix).expect("Failed to delete key.");
        if value.is_none() {
            continue;
        }
        let mut value = value.unwrap();
        for (deleted_key, deleted_value) in &value {
            assert_eq!(key_prefix, &deleted_key[0..key_prefix.len()]);
            assert_eq!(deleted_key, &vec![deleted_value.as_ref(); 2].concat());
        }

        for item in value.drain(..) {
            values.push(item);
        }

        let value = state.delete_all(key).expect("Failed to delete key.");
        assert_eq!(value, None);
    }

    let mut epoch_id = H256::default();
    epoch_id[0] = 2;
    let state_root = state.compute_state_root().unwrap();
    state.commit(epoch_id).unwrap();

    assert_eq!(values.len(), keys.len());
    assert_eq!(state_root, empty_state_root);
}

fn print(key: &[u8]) {
    print!("key = (");
    for char in key {
        print!(
            "{}, {}, ",
            CompressedPathRaw::first_nibble(*char),
            CompressedPathRaw::second_nibble(*char)
        );
    }
    println!(")");
}

#[test]
fn test_set_order() {
    let mut rng = get_rng_for_test();
    let state_manager = new_state_manager_for_testing();
    let keys: Vec<[u8; 4]> = generate_keys(500000)
        .iter()
        .filter(|_| rng.gen_bool(0.5))
        .cloned()
        .collect();

    let mut epoch_id = H256::default();
    let mut state_0 = state_manager.get_state_at(H256::default()).unwrap();
    println!("Setting state_0 with {} keys.", keys.len());
    for key in &keys {
        let key_slice = &key[..];
        let actual_key = vec![key_slice; 3].concat();
        let actual_value = vec![key_slice; (key[0] % 21) as usize].concat();
        state_0
            .set(&actual_key, &actual_value)
            .expect("Failed to insert key.");
    }
    let merkle_0 = state_0.compute_state_root().unwrap();
    epoch_id[0] = 1;
    state_0.commit(epoch_id).unwrap();

    let parent_epoch_0 = epoch_id;

    let mut state_1 = state_manager.get_state_at(parent_epoch_0).unwrap();
    println!("Setting state_1 with {} keys.", keys.len());
    for key in &keys {
        let key_slice = &key[..];
        let actual_key = vec![key_slice; 3].concat();
        let actual_value = vec![key_slice; (key[0] % 32) as usize].concat();
        state_1
            .set(&actual_key, &actual_value)
            .expect("Failed to insert key.");
    }
    let merkle_1 = state_1.compute_state_root().unwrap();
    epoch_id[0] = 2;
    state_1.commit(epoch_id).unwrap();

    let mut state_2 = state_manager.get_state_at(parent_epoch_0).unwrap();
    println!("Setting state_2 with {} keys.", keys.len());
    for key in keys.iter().rev() {
        let key_slice = &key[..];
        let actual_key = vec![key_slice; 3].concat();
        let actual_value = vec![key_slice; (key[0] % 32) as usize].concat();
        state_2
            .set(&actual_key, &actual_value)
            .expect("Failed to insert key.");
    }
    let merkle_2 = state_2.compute_state_root().unwrap();
    epoch_id[0] = 3;
    state_2.commit(epoch_id).unwrap();

    assert_eq!(merkle_1, merkle_2);
}

#[test]
fn test_set_order_concurrent() {
    let mut rng = get_rng_for_test();
    let state_manager = Arc::new(new_state_manager_for_testing());
    let keys = Arc::new(
        generate_keys(10000)
            .iter()
            .filter(|_| rng.gen_bool(0.5))
            .cloned()
            .collect::<Vec<_>>(),
    );

    let mut epoch_id = H256::default();
    let mut state_0 = state_manager.get_state_at(H256::default()).unwrap();
    println!("Setting state_0 with {} keys.", keys.len());
    for key in keys.iter() {
        let key_slice = &key[..];
        let actual_key = vec![key_slice; 3].concat();
        let actual_value = vec![key_slice; (key[0] % 21) as usize].concat();
        state_0
            .set(&actual_key, &actual_value)
            .expect("Failed to insert key.");
    }
    let merkle_0 = state_0.compute_state_root().unwrap();
    epoch_id[0] = 1;
    state_0.commit(epoch_id).unwrap();

    let parent_epoch_0 = epoch_id;

    let mut state_1 = state_manager.get_state_at(parent_epoch_0).unwrap();
    println!("Setting state_1 with {} keys.", keys.len());
    for key in keys.iter() {
        let key_slice = &key[..];
        let actual_key = vec![key_slice; 3].concat();
        let actual_value = vec![key_slice; (key[0] % 32) as usize].concat();
        state_1
            .set(&actual_key, &actual_value)
            .expect("Failed to insert key.");
    }
    let merkle_1 = state_1.compute_state_root().unwrap();
    epoch_id[0] = 2;
    state_1.commit(epoch_id).unwrap();

    const THREAD_COUNT: usize = 500;
    let mut threads = Vec::with_capacity(THREAD_COUNT);
    for thread_id in 0..THREAD_COUNT {
        thread::sleep_ms(30);
        let keys = keys.clone();
        let state_manager = state_manager.clone();
        threads.push(thread::spawn(move || {
            let mut state_2 =
                state_manager.get_state_at(parent_epoch_0).unwrap();
            println!(
                "Setting state_{} with {} keys.",
                2 + thread_id,
                keys.len()
            );
            for key in keys.iter().rev() {
                let key_slice = &key[..];
                let actual_key = vec![key_slice; 3].concat();
                let actual_value =
                    vec![key_slice; (key[0] % 32) as usize].concat();
                state_2
                    .set(&actual_key, &actual_value)
                    .expect("Failed to insert key.");
            }
            let merkle_2 = state_2.compute_state_root().unwrap();
            epoch_id[0] = ((3 + thread_id) % 256) as u8;
            epoch_id[1] = ((3 + thread_id) / 256) as u8;
            state_2.commit(epoch_id).unwrap();

            assert_eq!(merkle_1, merkle_2);
        }));
    }
    {
        let mut thread_id = 0;
        for thread in threads.drain(..) {
            thread
                .join()
                .expect(&format!("Thread {} failed.", thread_id));
            thread_id += 1;
        }
    }
}

use super::{
    super::{
        impls::multi_version_merkle_patricia_trie::merkle_patricia_trie::CompressedPathRaw,
        state::*, state_manager::*,
    },
    new_state_manager_for_testing,
};
use cfx_types::H256;
use rand::{ChaChaRng, Rng, SeedableRng};
use std::{mem, sync::Arc, thread};
