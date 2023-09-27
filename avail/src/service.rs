use core::time::Duration;

use anyhow::anyhow;
use avail_subxt::api;
use avail_subxt::api::runtime_types::sp_core::bounded::bounded_vec::BoundedVec;
use avail_subxt::primitives::AvailExtrinsicParams;
use avail_subxt::AvailConfig;
use reqwest::StatusCode;
use sp_core::crypto::Pair as PairTrait;
use sp_keyring::sr25519::sr25519::Pair;
use subxt::tx::PairSigner;
use subxt::OnlineClient;


use crate::avail::AvailBlobTransaction;
use crate::avail::AvailBlock;
use crate::avail::AvailHeader;
use crate::avail::{Confidence, ExtrinsicsData};

/// Runtime configuration for the DA service
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct DaServiceConfig {
    pub light_client_url: String,
    pub node_client_url: String,
    //TODO: Safer strategy to load seed so it is not accidentally revealed.
    pub seed: String,
    pub app_id: u32,
}

#[derive(Clone)]
pub struct DaProvider {
    pub node_client: OnlineClient<AvailConfig>,
    pub light_client_url: String,
    app_id: u32,
    signer: PairSigner<AvailConfig, Pair>,
}

impl DaProvider {
    fn appdata_url(&self, block_num: u64) -> String {
        let light_client_url = self.light_client_url.clone();
        format!("{light_client_url}/v1/appdata/{block_num}")
    }

    fn confidence_url(&self, block_num: u64) -> String {
        let light_client_url = self.light_client_url.clone();
        format!("{light_client_url}/v1/confidence/{block_num}")
    }

    pub async fn new(config: DaServiceConfig) -> Self {
        let pair = Pair::from_string_with_seed(&config.seed, None).unwrap();
        let signer = PairSigner::<AvailConfig, Pair>::new(pair.0.clone());

        let node_client = avail_subxt::build_client(config.node_client_url.to_string(), false)
            .await
            .unwrap();
        let light_client_url = config.light_client_url;

        DaProvider {
            node_client,
            light_client_url,
            signer,
            app_id: config.app_id
        }
    }
}

const POLLING_TIMEOUT: Duration = Duration::from_secs(60);
const POLLING_INTERVAL: Duration = Duration::from_secs(2);

async fn wait_for_confidence(confidence_url: &str) -> anyhow::Result<()> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= POLLING_TIMEOUT {
            return Err(anyhow!("Timeout..."));
        }

        let response = reqwest::get(confidence_url).await?;
        if response.status() != StatusCode::OK {
            println!("Confidence not received");
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        let response: Confidence = serde_json::from_str(&response.text().await?)?;
        if response.confidence < 92.5 {
            println!("Confidence not reached");
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        break;
    }

    Ok(())
}

async fn wait_for_appdata(appdata_url: &str, block: u32) -> anyhow::Result<ExtrinsicsData> {
    let start_time = std::time::Instant::now();

    loop {
        if start_time.elapsed() >= POLLING_TIMEOUT {
            return Err(anyhow!("Timeout..."));
        }

        let response = reqwest::get(appdata_url).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(ExtrinsicsData {
                block,
                extrinsics: vec![],
            });
        }
        if response.status() != StatusCode::OK {
            tokio::time::sleep(POLLING_INTERVAL).await;
            continue;
        }

        let appdata: ExtrinsicsData = serde_json::from_str(&response.text().await?)?;
        return Ok(appdata);
    }
}

impl DaProvider {
    // Make an RPC call to the node to get the finalized block at the given height, if one exists.
    // If no such block exists, block until one does.
    pub async fn get_finalized_at(&self, height: u64) -> Result<AvailBlock, anyhow::Error> {
        let node_client = self.node_client.clone();
        let confidence_url = self.confidence_url(height);
        let appdata_url = self.appdata_url(height);

        wait_for_confidence(&confidence_url).await?;
        let appdata = wait_for_appdata(&appdata_url, height as u32).await?;
        println!("Appdata: {:?}", appdata);

        let hash = node_client
            .rpc()
            .block_hash(Some(height.into()))
            .await?
            .unwrap();

        let header = node_client.rpc().header(Some(hash)).await?.unwrap();

        let header = AvailHeader::new(header, hash);
        let transactions = appdata
            .extrinsics
            .iter()
            .map(AvailBlobTransaction::new)
            .collect();
        Ok(AvailBlock {
            header,
            transactions,
        })
    }

    // Make an RPC call to the node to get the block at the given height
    // If no such block exists, block until one does.
    pub async fn get_block_at(&self, height: u64) -> Result<AvailBlock, anyhow::Error> {
        self.get_finalized_at(height).await
    }

    pub async fn send_transaction(&self, blob: &[u8]) -> Result<(), anyhow::Error> {
    println!("DStarted submissions");

      let data_transfer = api::tx()
      .data_availability()
      .submit_data(BoundedVec(blob.to_vec()));
    println!("created extrinsic");

      println!("blob {:?}", &blob);
        
      let extrinsic_params = AvailExtrinsicParams::new_with_app_id(self.app_id.into());
      println!("Signing and sending extrinsic to app id signer: {:#?}", &self.signer.account_id());

      let h = self.node_client
      .tx()
      .sign_and_submit_then_watch(&data_transfer, &self.signer, extrinsic_params)
      .await?;

      println!("Transaction submitted: {:#?}", h.extrinsic_hash());

      Ok(())
    }
}
