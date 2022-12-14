use crate::happy_path::test_erc20_deposit_panic;
use crate::happy_path_v2::deploy_cosmos_representing_erc20_and_check_adoption;
use crate::utils::{footoken_metadata, ValidatorKeys};
use crate::{one_eth, ADDRESS_PREFIX};
use deep_space::{Contact, CosmosPrivateKey, PrivateKey};
use gravity_proto::cosmos_sdk_proto::cosmos::bank::v1beta1::Metadata;
use tonic::transport::Channel;
use web30::client::Web3;

pub async fn send_to_eth_fees_test(
    web30: &Web3,
    gravity_client: GravityQueryClient<Channel>,
    contact: &Contact,
    keys: Vec<ValidatorKeys>,
    ibc_keys: Vec<CosmosPrivateKey>,
    gravity_address: EthAddress,
    erc20_addresses: Vec<EthAddress>,
) {
    let (ibc_metadata, ibc_erc20) =
        setup(web30, contact, keys, gravity_address, erc20_addresses).await;
    // Collect the denoms we will be using
    let mut test_denoms = vec![];
    test_denoms.push(ibc_metadata.base); // Add footoken
    for erc20 in erc20_addresses {
        test_denoms.push(erc20);
    }

    // TODO: Create a staker in addition to the validators
    // TODO: Send tokens lots of different ways to Ethereum
    // Try to create multi-Msg Txs as well to account for that
    // Send multiple denoms in a multi-Msg Tx
    // Send too little/exactly/more than the amount needed for one governance value
    // Update the governance value while sending more Msgs
    // Send too little/exactly/more than the amount needed for the new value
    // Disable the fees and check that no fees work moving forward
    // TODO: Check the balances of the staker + vals is correct in proportion to the total rewards
    // and the amount they have staked, in terms of each coin
}

pub async fn setup(
    web30: &Web3,
    contact: &Contact,
    keys: Vec<ValidatorKeys>,
    gravity_address: EthAddress,
    erc20_addresses: Vec<EthAddress>,
) -> (Metadata, EthAddress) {
    let ibc_metadata = footoken_metadata(contact).await;

    let new_erc20_contract = deploy_cosmos_representing_erc20_and_check_adoption(
        gravity_address,
        web30,
        Some(keys.clone()),
        &mut grpc_client,
        validator_out,
        ibc_metadata.clone(),
    )
    .await;

    // Send the validators generated address 100 units of each currency ethereum to cosmos
    for e in erc20_addresses {
        for v in keys {
            let receiver = v.validator_key.to_address(ADDRESS_PREFIX.as_str()).unwrap();
            test_erc20_deposit_panic(
                web30,
                contact,
                &mut grpc_client,
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

    (ibc_metadata, new_erc20_contract)
}
