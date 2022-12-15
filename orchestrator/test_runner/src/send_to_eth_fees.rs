use crate::airdrop_proposal::wait_for_proposals_to_execute;
use crate::happy_path::test_erc20_deposit_panic;
use crate::happy_path_v2::deploy_cosmos_representing_erc20_and_check_adoption;
use crate::utils::{
    footoken_metadata, get_user_key, vote_yes_on_proposals, BridgeUserKey, ValidatorKeys,
};
use crate::{
    get_deposit, get_fee, one_eth, ADDRESS_PREFIX, OPERATION_TIMEOUT, STAKING_TOKEN, TOTAL_TIMEOUT,
};
use actix::clock::sleep;
use clarity::Address as EthAddress;
use cosmos_gravity::proposals::{submit_send_to_eth_fees_proposal, SendToEthFeesProposalJson};
use cosmos_gravity::query::get_min_chain_fee_basis_points;
use cosmos_gravity::send::MSG_SEND_TO_ETH_TYPE_URL;
use deep_space::{Coin, Contact, CosmosPrivateKey, Fee, MessageArgs, Msg, PrivateKey};
use gravity_proto::cosmos_sdk_proto::cosmos::bank::v1beta1::Metadata;
use gravity_proto::gravity::{query_client::QueryClient as GravityQueryClient, MsgSendToEth};
use gravity_utils::num_conversion::one_atom;
use num::Integer;
use num256::Uint256;
use std::collections::HashMap;
use std::ops::{Add, Mul};
use std::time::Duration;
use tonic::transport::Channel;
use web30::client::Web3;

const BASIS_POINT_DIVISOR: u64 = 10000;
const GOVERNANCE_VOTING_PERIOD: u64 = 120; // This is set in tests/container-scripts/setup-validators.sh

pub async fn send_to_eth_fees_test(
    web30: &Web3,
    contact: &Contact,
    gravity_client: GravityQueryClient<Channel>,
    keys: Vec<ValidatorKeys>,
    gravity_address: EthAddress,
    erc20_addresses: Vec<EthAddress>,
) {
    let (ibc_metadata, staker_key) = setup(
        web30,
        contact,
        &gravity_client,
        keys.clone(),
        gravity_address,
        erc20_addresses.clone(),
    )
    .await;
    let ibc_denom = ibc_metadata.base;
    let erc20_denoms: Vec<String> = (&erc20_addresses)
        .into_iter()
        .map(|e| format!("gravity{}", e.to_string()))
        .collect();
    let (_staker_key, _staker_addr) = (staker_key.cosmos_key, staker_key.cosmos_address);
    // Collect the denoms we will be using
    let mut test_denoms = vec![];
    test_denoms.push(ibc_denom.clone()); // Add footoken
    for erc20 in erc20_addresses {
        test_denoms.push(erc20.to_string());
    }

    let val0_cosmos_key = keys[0].validator_key;
    let _val0_cosmos_addr = val0_cosmos_key.to_address(&ADDRESS_PREFIX).unwrap();
    let val0_eth_key = keys[0].eth_key;
    let val0_eth_addr = val0_eth_key.to_address();

    // Test the default fee config
    // TODO: Add assertions for collected rewards
    let _starting_fee_exp = send_to_eth_one_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;
    let _starting_fee_exp = send_to_eth_multi_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;

    // Change the fee config to our initial production value
    submit_and_pass_send_to_eth_fees_proposal(2, contact, &keys).await;
    // TODO: Add assertions for collected rewards
    let _mid_fee_exp = send_to_eth_one_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;
    let _mid_fee_exp = send_to_eth_multi_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;

    // Change the fee config while submitting sends to eth
    let _min_fee_exp = send_to_eth_while_changing_params(
        contact,
        &keys,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;

    // Change the fee config back to 0 and verify that the bridge still works
    submit_and_pass_send_to_eth_fees_proposal(0, contact, &keys).await;
    // TODO: Add assertions for collected rewards
    let _end_fee_exp = send_to_eth_one_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;
    let _end_fee_exp = send_to_eth_multi_msg_txs(
        contact,
        val0_cosmos_key,
        val0_eth_addr,
        erc20_denoms.clone(),
        ibc_denom.clone(),
    )
    .await;
}

/// Deploys the footoken erc20 representation, sends the Eth-native ERC20s to the cosmos validators,
/// and creates a staker who has delegations to every validator
pub async fn setup(
    web30: &Web3,
    contact: &Contact,
    grpc_client: &GravityQueryClient<Channel>,
    keys: Vec<ValidatorKeys>,
    gravity_address: EthAddress,
    erc20_addresses: Vec<EthAddress>,
) -> (Metadata, BridgeUserKey) {
    let ibc_metadata = footoken_metadata(contact).await;

    let _ = deploy_cosmos_representing_erc20_and_check_adoption(
        gravity_address,
        web30,
        Some(keys.clone()),
        &mut (grpc_client.clone()),
        false,
        ibc_metadata.clone(),
    )
    .await;

    // Send the validators generated address 100 units of each erc20 from ethereum to cosmos
    for e in erc20_addresses {
        for v in &keys {
            let receiver = v.validator_key.to_address(ADDRESS_PREFIX.as_str()).unwrap();
            test_erc20_deposit_panic(
                web30,
                contact,
                &mut (grpc_client.clone()),
                receiver,
                gravity_address,
                e,
                one_eth() * 100u64.into(),
                None,
                None,
            )
            .await;
        }
    }

    // Create a staker who should receive fee rewards too
    let staker_key = get_user_key(None);
    let staker_amt = Coin {
        denom: STAKING_TOKEN.clone(),
        amount: one_atom().mul(100u8.into()),
    };
    let zero_fee = Coin {
        denom: STAKING_TOKEN.clone(),
        amount: 0u8.into(),
    };
    for v in &keys {
        let res = contact
            .send_coins(
                staker_amt.clone(),
                None,
                staker_key.cosmos_address,
                Some(OPERATION_TIMEOUT),
                v.validator_key,
            )
            .await;
        info!("Sent coins to staker with response {:?}", res)
    }
    for v in &keys {
        let validator_addr = v
            .validator_key
            .to_address((ADDRESS_PREFIX.clone() + "valoper").as_str())
            .unwrap();
        let res = contact
            .delegate_to_validator(
                validator_addr,
                staker_amt.clone(),
                zero_fee.clone(),
                staker_key.cosmos_key.clone(),
                Some(OPERATION_TIMEOUT),
            )
            .await;
        info!("Delegated to validator with response {:?}", res)
    }

    (ibc_metadata, staker_key)
}

/// Helpful data for verifying that fee tests execute as expected
pub struct SendToEthFeeExpectations {
    pub expected_collected_fees: Vec<Coin>, // Fees to collect on expected successful txs
    pub expected_failed_fees: Vec<Coin>,    // Fees to fail to collect due to insufficient fees
    pub total_attempted_fees: Vec<Coin>,    // Total fees on all attempted txs
}

/// Creates a number of txs with each containing a single MsgSendToEth, with fee values informed by
/// the current gravity params
pub async fn send_to_eth_one_msg_txs(
    contact: &Contact,
    sender: CosmosPrivateKey,
    receiver: EthAddress,
    erc20_denoms: Vec<String>,
    ibc_denom: String,
) -> SendToEthFeeExpectations {
    let current_fee_basis_points = get_min_chain_fee_basis_points(contact).await;

    // Create the test transactions
    let (coins_to_send, fees_for_send, success_fee_tally, failure_fee_tally, total_fee_tally) =
        setup_transactions(current_fee_basis_points, erc20_denoms, ibc_denom, 1);

    // Send the transactions
    let sender_addr = sender
        .to_address(ADDRESS_PREFIX.as_str())
        .unwrap()
        .to_string();
    let tx_fee = Coin {
        denom: (*STAKING_TOKEN).clone(),
        amount: 0u8.into(),
    };
    for (bridge, fee) in coins_to_send.into_iter().zip(fees_for_send) {
        let fee_for_relayer = Coin {
            denom: bridge.denom.clone(),
            amount: 1u8.into(),
        };
        let msg_send_to_eth = MsgSendToEth {
            sender: sender_addr.clone(),
            eth_dest: receiver.to_string(),
            amount: Some(bridge.into()),
            bridge_fee: Some(fee_for_relayer.into()),
            chain_fee: Some(fee.into()),
        };

        let msg = Msg::new(MSG_SEND_TO_ETH_TYPE_URL, msg_send_to_eth);
        let _ = contact
            .send_message(
                &[msg],
                None,
                &[tx_fee.clone()],
                Some(OPERATION_TIMEOUT),
                sender.clone(),
            )
            .await;
    }

    // Return the expected results for verification
    SendToEthFeeExpectations {
        expected_collected_fees: fee_tally_to_coins(success_fee_tally),
        expected_failed_fees: fee_tally_to_coins(failure_fee_tally),
        total_attempted_fees: fee_tally_to_coins(total_fee_tally),
    }
}

/// Creates a number of txs with each containing multiple MsgSendToEth's, with fee values informed by
/// the current gravity params
pub async fn send_to_eth_multi_msg_txs(
    contact: &Contact,
    sender: CosmosPrivateKey,
    receiver: EthAddress,
    erc20_denoms: Vec<String>,
    ibc_denom: String,
) -> SendToEthFeeExpectations {
    let current_fee_basis_points = get_min_chain_fee_basis_points(contact).await;
    // Create the test transactions
    let (coins_to_send, fees_for_send, success_fee_tally, failure_fee_tally, total_fee_tally) =
        setup_transactions(
            current_fee_basis_points,
            erc20_denoms,
            ibc_denom,
            7, // Create lots of sends for us to pack into Txs
        );

    // Create and send Txs with multiple msgs in each tx
    let sender_addr = sender
        .to_address(ADDRESS_PREFIX.as_str())
        .unwrap()
        .to_string();
    let tx_fee = Coin {
        denom: (*STAKING_TOKEN).clone(),
        amount: 0u8.into(),
    };
    // A buffer of Msgs built iteratively before use with contact.send_message()
    let mut msgs = vec![];
    // The number of Msgs in our 1st, 2nd, 3rd, ... Txs
    let tx_sizes: Vec<usize> = vec![3, 10, 20, 30, 40, 60];
    let mut tx_size_idx = 0; // Our pointer into tx_sizes

    // Craft MsgSendToEth's, add them to the buffer, occasionally submit the Msgs in one Tx
    for (bridge, fee) in coins_to_send.into_iter().zip(fees_for_send) {
        let fee_for_relayer = Coin {
            denom: bridge.denom.clone(),
            amount: 1u8.into(),
        };
        let msg_send_to_eth = MsgSendToEth {
            sender: sender_addr.clone(),
            eth_dest: receiver.to_string(),
            amount: Some(bridge.into()),
            bridge_fee: Some(fee_for_relayer.into()),
            chain_fee: Some(fee.into()),
        };

        let msg = Msg::new(MSG_SEND_TO_ETH_TYPE_URL, msg_send_to_eth);
        msgs.push(msg);

        if msgs.len() >= *tx_sizes.get(tx_size_idx).unwrap() {
            let to_send = msgs.clone();
            msgs = vec![];
            if tx_size_idx < tx_sizes.len() - 1 {
                tx_size_idx += 1;
            }
            let _ = contact
                .send_message(
                    &to_send,
                    None,
                    &[tx_fee.clone()],
                    Some(OPERATION_TIMEOUT),
                    sender.clone(),
                )
                .await;
        }
    }

    // Return the expected results for verification
    SendToEthFeeExpectations {
        expected_collected_fees: fee_tally_to_coins(success_fee_tally),
        expected_failed_fees: fee_tally_to_coins(failure_fee_tally),
        total_attempted_fees: fee_tally_to_coins(total_fee_tally),
    }
}

/// Creates a number of sends to Ethereum while changing the minimum fees up at the same time
pub async fn send_to_eth_while_changing_params(
    contact: &Contact,
    keys: &[ValidatorKeys],
    sender: CosmosPrivateKey,
    receiver: EthAddress,
    erc20_denoms: Vec<String>,
    ibc_denom: String,
) -> SendToEthFeeExpectations {
    let current_fee_basis_points = get_min_chain_fee_basis_points(contact).await;

    assert!(
        current_fee_basis_points != 0,
        "send_to_eth_while_changing_params requires that the current fee be configured!"
    );
    // Create the test transactions
    let (mut coins_to_send, mut fees_for_send, _, _, _) = setup_transactions(
        current_fee_basis_points,
        erc20_denoms.clone(),
        ibc_denom.clone(),
        7, // Create lots of sends for us to pack into Txs
    );
    let (mut coins_to_send2, mut fees_for_send2, min_success_fee_tally, min_failure_fee_tally, _) =
        setup_transactions(
            current_fee_basis_points * 10, // 10x the required fee amounts
            erc20_denoms.clone(),
            ibc_denom.clone(),
            7, // Create lots of sends for us to pack into Txs
        );
    // Add the txs with higher fee to the end
    coins_to_send.append(&mut coins_to_send2);
    fees_for_send.append(&mut fees_for_send2);

    // Create and send Txs with multiple msgs in each tx
    let sender_address = sender.to_address(ADDRESS_PREFIX.as_str()).unwrap();
    let sender_addr = sender_address.to_string();

    // We pay 100 stake on the first tx, then 99 on 2nd, ... to encourage sequential ordering,
    // otherwise our Txs could fail due to a Tx with sequence number 3 executing before the Tx with sequence 2
    let some_stake = Coin {
        denom: (*STAKING_TOKEN).clone(),
        amount: 100u16.into(),
    };
    let args_fee = Fee {
        amount: vec![some_stake],
        gas_limit: 400_000,
        granter: None,
        payer: None,
    };
    // A template for our MessageArgs, we need to change the sequence and fee amount each time we call contact.send_message()
    let args_tmpl8 = contact
        .get_message_args(sender_address, args_fee.clone())
        .await
        .unwrap();

    // An effective queue of (MessageArgs, Vec<Msg>) for contact.send_message()
    let mut msg_args_queue = vec![];
    let mut msgs = vec![]; // A Msg buffer built iteratively and added to msg_args_queue

    // The number of txs to have in the 1st, 2nd, 3rd, ... Txs we submit
    let tx_sizes: Vec<usize> = vec![2, 4, 6, 5, 8, 3, 1, 3, 5, 4, 9, 12, 9, 1];
    let mut tx_size_idx = 0; // Our pointer into tx_sizes

    // Create a queue of multi-Msg Tx's to send later on
    for (bridge, fee) in coins_to_send.into_iter().zip(fees_for_send) {
        let fee_for_relayer = Coin {
            denom: bridge.denom.clone(),
            amount: 1u8.into(),
        };
        let msg_send_to_eth = MsgSendToEth {
            sender: sender_addr.clone(),
            eth_dest: receiver.to_string(),
            amount: Some(bridge.into()),
            bridge_fee: Some(fee_for_relayer.into()),
            chain_fee: Some(fee.into()),
        };

        let msg = Msg::new(MSG_SEND_TO_ETH_TYPE_URL, msg_send_to_eth);
        msgs.push(msg);

        let msgs_for_this_tx = *tx_sizes.get(tx_size_idx).unwrap();
        if msgs.len() >= msgs_for_this_tx {
            let to_send = msgs.clone();
            let tx_num: u64 = msg_args_queue.len() as u64; // which tx this is

            // Create a MessageArgs which will help prioritize our Tx in the mempool
            let mut args = args_tmpl8.clone();
            args.sequence += tx_num; // Fix sequence to avoid tx rejection
            args.fee.amount[0].amount -= tx_num.into(); // Pay less for sequential ordering

            msg_args_queue.push((args, to_send));

            if tx_size_idx < tx_sizes.len() - 1 {
                tx_size_idx += 1;
            }
        }
    }

    let _ = submit_send_to_eth_fees_proposal(
        SendToEthFeesProposalJson {
            title: "Send to eth fees".to_string(),
            description: "send to eth fees".to_string(),
            min_chain_fee_basis_points: current_fee_basis_points * 10,
        },
        Coin {
            denom: (*STAKING_TOKEN).clone(),
            amount: 1000u32.into(),
        },
        Coin {
            denom: (*STAKING_TOKEN).clone(),
            amount: 0u32.into(),
        },
        contact,
        sender,
        Some(OPERATION_TIMEOUT),
    )
    .await;
    vote_yes_on_proposals(contact, keys, Some(OPERATION_TIMEOUT)).await;
    let delay = Duration::from_secs(GOVERNANCE_VOTING_PERIOD - 10);
    sleep(delay).await;

    execute_queued_msgs(contact, msg_args_queue, sender).await;

    // Return the minimum expected results for verification
    SendToEthFeeExpectations {
        expected_collected_fees: fee_tally_to_coins(min_success_fee_tally),
        expected_failed_fees: fee_tally_to_coins(min_failure_fee_tally),
        total_attempted_fees: vec![],
    }
}

pub async fn execute_queued_msgs(
    contact: &Contact,
    msg_args_queue: Vec<(MessageArgs, Vec<Msg>)>,
    sender: impl PrivateKey,
) {
    for (args, msgs) in msg_args_queue {
        let _ = contact
            .send_message_with_args(&msgs, None, args, Some(OPERATION_TIMEOUT), sender.clone())
            .await;
    }
}

fn setup_transactions(
    min_fee_basis_points: u64,
    erc20_denoms: Vec<String>,
    ibc_denom: String,
    tests_per_fee_amount: usize,
) -> (
    Vec<Coin>,
    Vec<Coin>,
    HashMap<String, Uint256>,
    HashMap<String, Uint256>,
    HashMap<String, Uint256>,
) {
    let mut success_fee_tally: HashMap<String, Uint256> = HashMap::new();
    let mut failure_fee_tally: HashMap<String, Uint256> = HashMap::new();
    let mut total_fee_tally: HashMap<String, Uint256> = HashMap::new();

    // Convert the minimum fee param to a useable fee for a send of one eth (1 * 10^18)
    let curr_fee_basis_points = Uint256::from(min_fee_basis_points);
    let erc20_bridge_amount: Uint256 = one_eth();
    let erc20_min_fee = get_min_fee(erc20_bridge_amount.clone(), curr_fee_basis_points.clone());
    let erc20_success_fees: Vec<Uint256> = get_success_test_fees(erc20_min_fee.clone());
    let erc20_fail_fees: Vec<Uint256> = get_fail_test_fees(erc20_min_fee.clone());

    // ... and for a send of one atom (1 * 10^6)
    let ibc_bridge_amount = one_atom();
    let ibc_min_fee = get_min_fee(ibc_bridge_amount.clone(), curr_fee_basis_points.clone());
    let ibc_success_fees: Vec<Uint256> = get_success_test_fees(ibc_min_fee.clone());
    let ibc_fail_fees: Vec<Uint256> = get_fail_test_fees(ibc_min_fee.clone());

    let mut coins_to_send = vec![];
    let mut fees_for_send = vec![];

    // Create some footoken success test cases from the above generated values
    queue_sends_to_eth(
        tests_per_fee_amount,
        ibc_denom.clone(),
        ibc_bridge_amount.clone(),
        ibc_success_fees,
        &mut coins_to_send,
        &mut fees_for_send,
        &mut success_fee_tally,
        &mut total_fee_tally,
    );
    // Create some footoken failure test cases from the above generated values
    queue_sends_to_eth(
        tests_per_fee_amount,
        ibc_denom.clone(),
        ibc_bridge_amount.clone(),
        ibc_fail_fees,
        &mut coins_to_send,
        &mut fees_for_send,
        &mut failure_fee_tally,
        &mut total_fee_tally,
    );
    // Create some erc20 success test cases from the above generated values
    for e in erc20_denoms {
        queue_sends_to_eth(
            tests_per_fee_amount,
            e.clone(),
            erc20_bridge_amount.clone(),
            erc20_success_fees.clone(),
            &mut coins_to_send,
            &mut fees_for_send,
            &mut success_fee_tally,
            &mut total_fee_tally,
        );

        // Create some erc20 failure test cases from the above generated values
        queue_sends_to_eth(
            tests_per_fee_amount,
            e.clone(),
            erc20_bridge_amount.clone(),
            erc20_fail_fees.clone(),
            &mut coins_to_send,
            &mut fees_for_send,
            &mut failure_fee_tally,
            &mut total_fee_tally,
        );
    }

    (
        coins_to_send,
        fees_for_send,
        success_fee_tally,
        failure_fee_tally,
        total_fee_tally,
    )
}

/// Adds the desired test cases to their respective Vec's and the fee tally hashmaps
/// More sends to eth will be queued if tests_per_fee_amount is > 1
#[allow(clippy::too_many_arguments)]
fn queue_sends_to_eth(
    tests_per_fee_amount: usize,
    denom: String,
    bridge_amount: Uint256,
    fee_amounts: Vec<Uint256>,
    bridge_collection: &mut Vec<Coin>,
    fee_collection: &mut Vec<Coin>,
    fee_tally: &mut HashMap<String, Uint256>,
    total_fee_tally: &mut HashMap<String, Uint256>,
) {
    for fee in fee_amounts {
        let fee = fee.clone();
        for _ in 0..tests_per_fee_amount {
            bridge_collection.push(Coin {
                denom: denom.clone(),
                amount: bridge_amount.clone(),
            });
            fee_collection.push(Coin {
                denom: denom.clone(),
                amount: fee.clone(),
            });
            fee_tally.insert(
                denom.clone(),
                fee_tally
                    .get(&denom)
                    .unwrap_or(&0u8.into())
                    .clone()
                    .add(fee.clone()),
            );
            total_fee_tally.insert(
                denom.clone(),
                total_fee_tally
                    .get(&denom)
                    .unwrap_or(&0u8.into())
                    .clone()
                    .add(fee.clone()),
            );
        }
    }
}

pub fn get_min_fee(bridge_amount: Uint256, min_fee_basis_points: Uint256) -> Uint256 {
    Uint256(
        bridge_amount
            .clone()
            .div_floor(&Uint256::from(BASIS_POINT_DIVISOR).0)
            .mul(min_fee_basis_points.0),
    )
}

pub fn get_success_test_fees(min_fee: Uint256) -> Vec<Uint256> {
    if min_fee == 0u8.into() {
        vec![0u8.into(), 1u8.into()]
    } else {
        vec![min_fee.clone(), min_fee + 10u8.into()]
    }
}

pub fn get_fail_test_fees(min_fee: Uint256) -> Vec<Uint256> {
    if min_fee == 0u8.into() {
        vec![]
    } else {
        vec![min_fee - 1u8.into()]
    }
}

pub fn fee_tally_to_coins(fee_tally: HashMap<String, Uint256>) -> Vec<Coin> {
    fee_tally
        .into_iter()
        .map(|(denom, amount)| Coin {
            denom: denom.to_string(),
            amount: amount.clone(),
        })
        .collect::<Vec<Coin>>()
}

pub async fn submit_and_pass_send_to_eth_fees_proposal(
    min_chain_fee_basis_points: u64,
    contact: &Contact,
    keys: &[ValidatorKeys],
) {
    let proposal_content = SendToEthFeesProposalJson {
        title: format!(
            "Set MinChainFeeBasisPoints to {}",
            min_chain_fee_basis_points
        ),
        description: "MinChainFeeBasisPoints!".to_string(),
        min_chain_fee_basis_points,
    };
    let res = submit_send_to_eth_fees_proposal(
        proposal_content,
        get_deposit(),
        get_fee(None),
        contact,
        keys[0].validator_key,
        Some(TOTAL_TIMEOUT),
    )
    .await;
    vote_yes_on_proposals(contact, keys, None).await;
    wait_for_proposals_to_execute(contact).await;
    trace!("Gov proposal executed with {:?}", res);

    let set_value = get_min_chain_fee_basis_points(contact).await;
    assert_eq!(
        min_chain_fee_basis_points, set_value,
        "actual value {} is not what was voted on {}",
        set_value, min_chain_fee_basis_points
    );
}
