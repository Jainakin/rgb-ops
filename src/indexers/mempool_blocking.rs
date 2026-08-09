// RGB ops library for working with smart contracts on Bitcoin & Lightning
//
// SPDX-License-Identifier: Apache-2.0
//
// Written in 2019-2023 by
//     Dr Maxim Orlovsky <orlovsky@lnp-bp.org>
//
// Copyright (C) 2019-2023 LNP/BP Standards Association. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use esplora_client::BlockingClient;
use rgb::bitcoin::Txid;
use rgbcore::validation::{ResolveWitness, WitnessResolverError, WitnessStatus};
use rgbcore::ChainNet;

use crate::indexers::esplora_blocking::esplora_client::Builder;
use crate::indexers::esplora_blocking::EsploraClient;

/// Wrapper of an esplora client, necessary to implement the foreign `ResolveWitness` trait.
/// It assumes that mempool.space exposes the same APIs as esplora.
// Currently, this client is wrapping an `crate::indexers::esplora_blocking::EsploraClient`
// instance. If the mempool service changes in the future and is not compatible with
// esplora::BlockingClient, only the internal implementation needs to be modified.
pub struct MemPoolClient {
    inner: EsploraClient,
}

impl MemPoolClient {
    /// Creates a new `MemPoolClient` instance.
    ///
    /// # Arguments
    ///
    /// * `builder` - The builder for the mempool client.
    ///
    /// # Returns
    ///
    /// Returns the `MemPoolClient` instance.
    #[allow(clippy::result_large_err)]
    pub fn new(builder: Builder) -> Self {
        let inner = EsploraClient {
            inner: BlockingClient::from_builder(builder),
        };
        MemPoolClient { inner }
    }
}

impl ResolveWitness for MemPoolClient {
    fn check_chain_net(&self, chain_net: ChainNet) -> Result<(), WitnessResolverError> {
        self.inner.check_chain_net(chain_net)
    }

    fn resolve_witness(&self, txid: Txid) -> Result<WitnessStatus, WitnessResolverError> {
        self.inner.resolve_witness(txid)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod test {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use super::*;

    const EXTERNAL_TEST_TIMEOUT_SECS: u64 = 30;

    fn external_builder(url: &str) -> Builder {
        Builder::new(url).timeout(EXTERNAL_TEST_TIMEOUT_SECS)
    }

    fn mock_builder(content_type: &'static str, body: Vec<u8>) -> (Builder, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock Esplora server");
        let address = listener.local_addr().expect("read mock server address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept mock Esplora request");
            let mut request = [0_u8; 4096];
            let bytes_read = stream
                .read(&mut request)
                .expect("read mock Esplora request");
            let request = String::from_utf8_lossy(&request[..bytes_read]).into_owned();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: \
                 {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("write mock Esplora response headers");
            stream
                .write_all(&body)
                .expect("write mock Esplora response body");
            request
        });
        let url = format!("http://{address}");
        (Builder::new(&url).timeout(5), handle)
    }

    #[test]
    fn mempool_client_parses_esplora_status_response() {
        let body = br#"{"confirmed":true,"block_height":1,"block_hash":"0000000000000000000000000000000000000000000000000000000000000000","block_time":1231006505}"#.to_vec();
        let (builder, server) = mock_builder("application/json", body);
        let client = super::MemPoolClient::new(builder);
        let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
            .parse()
            .unwrap();

        let status = client.inner.inner.get_tx_status(&txid).unwrap();

        assert_eq!(status.block_height, Some(1));
        assert_eq!(status.block_time, Some(1231006505));
        let request = server.join().expect("join mock Esplora server");
        assert!(request.starts_with(&format!("GET /tx/{txid}/status HTTP/1.1")));
    }

    #[test]
    fn mempool_client_parses_esplora_raw_transaction_response() {
        let genesis =
            rgb::bitcoin::blockdata::constants::genesis_block(rgb::bitcoin::Network::Bitcoin);
        let transaction = genesis.txdata[0].clone();
        let txid = transaction.compute_txid();
        let body = rgb::bitcoin::consensus::serialize(&transaction);
        let (builder, server) = mock_builder("application/octet-stream", body);
        let client = super::MemPoolClient::new(builder);

        let resolved = client
            .inner
            .inner
            .get_tx(&txid)
            .expect("parse mock transaction")
            .expect("mock transaction exists");

        assert_eq!(resolved, transaction);
        let request = server.join().expect("join mock Esplora server");
        assert!(request.starts_with(&format!("GET /tx/{txid}/raw HTTP/1.1")));
    }

    #[test]
    #[ignore = "requires live public mempool.space availability"]
    fn test_mempool_client_mainnet_tx() {
        let builder = external_builder("https://mempool.space/api");
        let client = super::MemPoolClient::new(builder);
        let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
            .parse()
            .unwrap();
        let status = client.inner.inner.get_tx_status(&txid).unwrap();
        assert_eq!(status.block_height, Some(0));
        assert_eq!(status.block_time, Some(1231006505));
    }

    #[test]
    #[ignore = "requires live public mempool.space availability"]
    fn test_mempool_client_testnet_tx() {
        let builder = external_builder("https://mempool.space/testnet/api");
        let client = super::MemPoolClient::new(builder);

        let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
            .parse()
            .unwrap();
        let status = client.inner.inner.get_tx_status(&txid).unwrap();
        assert_eq!(status.block_height, Some(0));
        assert_eq!(status.block_time, Some(1296688602));
    }

    #[test]
    #[ignore = "requires live public mempool.space availability"]
    fn test_mempool_client_testnet4_tx() {
        let builder = external_builder("https://mempool.space/testnet4/api");
        let client = super::MemPoolClient::new(builder);
        let txid = "7aa0a7ae1e223414cb807e40cd57e667b718e42aaf9306db9102fe28912b7b4e"
            .parse()
            .unwrap();
        let status = client.inner.inner.get_tx_status(&txid).unwrap();
        assert_eq!(status.block_height, Some(0));
        assert_eq!(status.block_time, Some(1714777860));
    }

    #[test]
    #[ignore = "requires live public mempool.space availability"]
    fn test_mempool_client_testnet4_tx_detail() {
        let builder = external_builder("https://mempool.space/testnet4/api");
        let client = super::MemPoolClient::new(builder);
        let txid = "7aa0a7ae1e223414cb807e40cd57e667b718e42aaf9306db9102fe28912b7b4e"
            .parse()
            .unwrap();
        let tx = client
            .inner
            .inner
            .get_tx(&txid)
            .expect("Failed to get tx")
            .expect("Tx not found");
        assert!(!tx.input.is_empty());
        assert!(!tx.output.is_empty());
        assert_eq!(tx.output[0].value.to_sat(), 5_000_000_000);
    }
}
