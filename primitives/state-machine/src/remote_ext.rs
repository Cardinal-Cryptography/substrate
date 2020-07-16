use super::TestExternalities;
use log::*;
use sp_core::hashing::twox_128;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use sub_storage::StorageKey;

type Hash = sp_core::H256;

macro_rules! wait {
	($e:expr) => {
		async_std::task::block_on($e)
	};
}

const LOG_TARGET: &'static str = "remote-ext";

/// Struct for better hex printing of slice types.
pub struct HexSlice<'a>(&'a [u8]);

impl<'a> HexSlice<'a> {
	/// Create a new HexSlice.
	pub fn new<T>(data: &'a T) -> HexSlice<'a>
	where
		T: ?Sized + AsRef<[u8]> + 'a,
	{
		HexSlice(data.as_ref())
	}
}

// You can choose to implement multiple traits, like Lower and UpperHex
impl Debug for HexSlice<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		write!(f, "0x")?;
		for byte in self.0 {
			write!(f, "{:x}", byte)?;
		}
		Ok(())
	}
}

/// Extension trait for hex display.
pub trait HexDisplayExt {
	/// Display self as hex.
	fn hex_display(&self) -> HexSlice<'_>;
}

impl<T: ?Sized + AsRef<[u8]>> HexDisplayExt for T {
	fn hex_display(&self) -> HexSlice<'_> {
		HexSlice::new(self)
	}
}

/// Builder for remote-externalities.
#[derive(Debug, Default)]
pub struct Builder {
	at: Option<Hash>,
	uri: Option<String>,
	inject: Vec<(Vec<u8>, Vec<u8>)>,
	module_filter: Vec<String>,
}

impl Builder {
	/// Create a new builder.
	pub fn new() -> Self {
		Default::default()
	}

	/// Scrape the chain at the given block hash.
	///
	/// If not set, latest finalized will be used.
	pub fn at(mut self, at: Hash) -> Self {
		self.at = Some(at);
		self
	}

	/// Look for a chain at the given URI.
	///
	/// If not set, `ws://localhost:9944` will be used.
	pub fn uri(mut self, uri: String) -> Self {
		self.uri = Some(uri);
		self
	}

	/// Inject a manual list of key and values to the storage.
	pub fn inject(mut self, injections: &[(Vec<u8>, Vec<u8>)]) -> Self {
		for i in injections {
			self.inject.push(i.clone());
		}
		self
	}

	/// Scrape only this module.
	///
	/// If used multiple times, all of the given modules will be used, else the entire chain.
	pub fn module(mut self, module: &str) -> Self {
		self.module_filter.push(module.to_string());
		self
	}

	/// Build the test externalities.
	pub fn build(self) -> TestExternalities<sp_core::Blake2Hasher> {
		let mut ext = TestExternalities::new_empty();
		let uri = self.uri.unwrap_or(String::from("ws://localhost:9944"));

		let transport = wait!(jsonrpsee::transport::ws::WsTransportClient::new(&uri))
			.expect("Failed to connect to client");
		let client: jsonrpsee::Client = jsonrpsee::raw::RawClient::new(transport).into();

		let head = wait!(sub_storage::get_head(&client));
		let at = self.at.unwrap_or(head);

		info!(target: LOG_TARGET, "connecting to node {} at {:?}", uri, at);

		let keys_and_values = if self.module_filter.len() > 0 {
			let mut filtered_kv = vec![];
			for f in self.module_filter {
				let hashed_prefix = twox_128(f.as_bytes());
				debug!(
					target: LOG_TARGET,
					"Downloading data for module {} (prefix: {:?}).",
					f,
					hashed_prefix.hex_display()
				);
				let module_kv = wait!(sub_storage::get_pairs(
					StorageKey(hashed_prefix.to_vec()),
					&client,
					at
				));

				for kv in module_kv.into_iter().map(|(k, v)| (k.0, v.0)) {
					filtered_kv.push(kv);
				}
			}
			filtered_kv
		} else {
			debug!(target: LOG_TARGET, "Downloading data for all modules.");
			wait!(sub_storage::get_pairs(
				StorageKey(Default::default()),
				&client,
				at
			))
			.into_iter()
			.map(|(k, v)| (k.0, v.0))
			.collect::<Vec<_>>()
		};

		info!(target: LOG_TARGET, "Done with scraping data ({} keys). Injecting.", keys_and_values.len());

		// inject all the scraped keys and values.
		for (k, v) in keys_and_values {
			trace!(
				target: LOG_TARGET,
				"injecting {:?} -> {:?}",
				k.hex_display(),
				v.hex_display()
			);
			ext.insert(k, v);
		}

		// lastly, insert the injections, if any.
		for (k, v) in self.inject.into_iter() {
			ext.insert(k, v);
		}

		info!(target: LOG_TARGET, "Done. Executing closure.");
		ext
	}
}