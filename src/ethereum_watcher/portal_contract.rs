use super::{ETHEREUM_BLOCK_TIME, ETHEREUM_CONNECTION_RETRIES};
use crate::WatchtowerConfig;

use anyhow::Result;
use ethers::abi::Address;
use ethers::prelude::k256::ecdsa::SigningKey;
use ethers::prelude::{abigen, SignerMiddleware};
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{Signer, Wallet};
use ethers::types::{Filter, H160, U256};
use std::cmp::max;
use std::convert::TryFrom;
use std::ops::Mul;
use std::str::FromStr;
use std::sync::Arc;

abigen!(FuelMessagePortal, "./abi/FuelMessagePortal.json");

#[derive(Clone, Debug)]
pub struct PortalContract {
    provider: Provider<Http>,
    contract: FuelMessagePortal<SignerMiddleware<Provider<Http>, Wallet<SigningKey>>>,
    address: H160,
    read_only: bool,
}

impl PortalContract {
    pub async fn new(config: &WatchtowerConfig) -> Result<Self> {
        // setup provider
        let provider = Provider::<Http>::try_from(&config.ethereum_rpc)?;
        let chain_id = provider.get_chainid().await?.as_u64();

        // setup wallet
        let mut read_only = false;
        let key_str = match &config.ethereum_wallet_key {
            Some(key) => key.clone(),
            None => {
                read_only = true;
                String::from("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
            }
        };
        let wallet: Wallet<SigningKey> = key_str.parse::<Wallet<SigningKey>>()?.with_chain_id(chain_id);

        // setup contract
        let address = Address::from_str(&config.portal_contract_address)?;
        let client = SignerMiddleware::new(provider.clone(), wallet);
        let contract = FuelMessagePortal::new(address, Arc::new(client));

        // verify contract setup is valid
        let contract_result = contract.paused().call().await;
        match contract_result {
            Err(_) => Err(anyhow::anyhow!("Invalid portal contract.")),
            Ok(_) => Ok(PortalContract {
                provider,
                contract,
                address,
                read_only,
            }),
        }
    }

    pub async fn get_amount_deposited(&self, timeframe: u32, latest_block_num: u64) -> Result<U256> {
        let block_offset = timeframe as u64 / ETHEREUM_BLOCK_TIME;
        let start_block = max(latest_block_num, block_offset) - block_offset;

        //MessageSent(bytes32 indexed sender, bytes32 indexed recipient, uint256 indexed nonce, uint64 amount, bytes data)
        let filter = Filter::new()
            .address(self.address)
            .event("MessageSent(bytes32,bytes32,uint256,uint64,bytes)")
            .from_block(start_block);
        for i in 0..ETHEREUM_CONNECTION_RETRIES {
            match self.provider.get_logs(&filter).await {
                Ok(logs) => {
                    let mut total = U256::zero();
                    for log in logs {
                        let amount = U256::from_big_endian(&log.data[0..32]).mul(U256::from(1_000_000_000));
                        total += amount;
                    }
                    return Ok(total);
                }
                Err(e) => {
                    if i == ETHEREUM_CONNECTION_RETRIES - 1 {
                        return Err(anyhow::anyhow!("{e}"));
                    }
                }
            }
        }
        Ok(U256::zero())
    }

    pub async fn get_amount_withdrawn(&self, timeframe: u32, latest_block_num: u64) -> Result<U256> {
        let block_offset = timeframe as u64 / ETHEREUM_BLOCK_TIME;
        let start_block = max(latest_block_num, block_offset) - block_offset;

        //MessageRelayed(bytes32 indexed messageId, bytes32 indexed sender, bytes32 indexed recipient, uint64 amount)
        let filter = Filter::new()
            .address(self.address)
            .event("MessageRelayed(bytes32,bytes32,bytes32,uint64)")
            .from_block(start_block);
        for i in 0..ETHEREUM_CONNECTION_RETRIES {
            match self.provider.get_logs(&filter).await {
                Ok(logs) => {
                    let mut total = U256::zero();
                    for log in logs {
                        let amount = U256::from_big_endian(&log.data[0..32]).mul(U256::from(1_000_000_000));
                        total += amount;
                    }
                    return Ok(total);
                }
                Err(e) => {
                    if i == ETHEREUM_CONNECTION_RETRIES - 1 {
                        return Err(anyhow::anyhow!("{e}"));
                    }
                }
            }
        }
        Ok(U256::zero())
    }

    pub async fn pause(&self) -> Result<()> {
        if self.read_only {
            return Err(anyhow::anyhow!("Ethereum account not configured."));
        }

        // TODO: implement alert on timeout and a gas escalator (https://github.com/gakonst/ethers-rs/blob/master/examples/middleware/examples/gas_escalator.rs)
        let result = self.contract.pause().call().await;
        match result {
            Err(e) => Err(anyhow::anyhow!("Failed to pause portal contract: {}", e)),
            Ok(_) => Ok(()),
        }
    }
}
