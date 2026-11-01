//! Bounded, replayable responses. Payloads are stored only in the SDK's
//! encrypted crypto store, and removed through a durable deletion tombstone.
use super::*;

const CACHE_LIMIT: usize = 32 * 1024 * 1024;

#[cfg(test)]
#[path = "responder_tests.rs"]
mod tests;

impl Recovery {
    pub(super) async fn prune(&mut self) -> Result<(), Failure> {
        self.ledger
            .rooms
            .retain(|_, p| p.unresolved || p.expires > now());
        self.ledger.completed.retain(|_, c| c.expires > now());
        self.ledger
            .responded
            .retain(|_, last| now().checked_sub(*last).is_none_or(|elapsed| elapsed < 60));
        let deletions: Vec<_> = self
            .ledger
            .cached
            .iter()
            .filter(|(_, c)| c.state == CacheState::Deleting || c.expires <= now())
            .map(|(k, _)| k.clone())
            .collect();
        for key in deletions {
            self.delete_cached(&key).await?;
        }
        self.ledger
            .cached
            .retain(|_, c| !(c.state == CacheState::Tombstone && c.expires <= now()));
        self.save().await
    }

    async fn delete_cached(&mut self, key: &str) -> Result<(), Failure> {
        let Some(cached) = self.ledger.cached.get_mut(key) else {
            return Ok(());
        };
        cached.state = CacheState::Deleting;
        let storage_key = cached.storage_key();
        self.save().await?;
        if self.store_value(&storage_key, &[]).await.is_err() {
            self.poisoned.store(true, Ordering::Release);
            return Err(Failure::Storage);
        }
        let cached = self.ledger.cached.get_mut(key).ok_or(Failure::Storage)?;
        cached.state = CacheState::Tombstone;
        cached.bytes = 0;
        self.save().await
    }

    pub(super) async fn acknowledged(
        &mut self,
        room: String,
        nonce: String,
        response: String,
        pin: Pin,
    ) -> Result<(), Failure> {
        let key = nonce_key(&pin, &nonce);
        let cached = self
            .ledger
            .cached
            .get(&key)
            .ok_or(Failure::InvalidResponse)?;
        if cached.pin != pin || cached.room != room || cached.response != response {
            return Err(Failure::InvalidResponse);
        }
        if cached.state == CacheState::Tombstone {
            return Ok(());
        }
        self.diagnostics.acknowledged_responses =
            self.diagnostics.acknowledged_responses.saturating_add(1);
        self.delete_cached(&key).await
    }

    pub(super) async fn respond(
        &mut self,
        room: String,
        nonce: String,
        expires: u64,
        device: Device,
        pin: Pin,
    ) -> Result<(), Failure> {
        let facts = self.facts(&room).await?;
        if !facts.joined.contains(&pin.user) {
            return Err(Failure::MembershipChanged);
        }
        let rate_key = format!("{room}\n{}", pin.user);
        if self
            .ledger
            .responded
            .get(&rate_key)
            .is_some_and(|last| now().checked_sub(*last).is_none_or(|elapsed| elapsed < 60))
        {
            return Err(Failure::RateLimited);
        }
        self.prune().await?;
        if self.ledger.responded.len() >= 512 && !self.ledger.responded.contains_key(&rate_key) {
            let _ = self
                .send(
                    &[device],
                    &Wire::Refused {
                        room,
                        nonce,
                        reason: Failure::RateLimited,
                    },
                )
                .await;
            return Err(Failure::RateLimited);
        }
        self.ledger.responded.insert(rate_key, now());
        self.save().await?;
        let key = nonce_key(&pin, &nonce);
        let result = self
            .prepare_or_replay(&key, &room, &nonce, expires, &device, &pin)
            .await;
        if let Err(reason) = result {
            if !self.poisoned.load(Ordering::Acquire)
                && self
                    .ledger
                    .cached
                    .get(&key)
                    .is_some_and(|c| c.state == CacheState::Preparing)
            {
                self.delete_cached(&key).await?;
            }
            if reason != Failure::RateLimited && !self.poisoned.load(Ordering::Acquire) {
                let _ = self
                    .send(
                        &[device],
                        &Wire::Refused {
                            room,
                            nonce,
                            reason,
                        },
                    )
                    .await;
            }
        }
        result
    }

    async fn prepare_or_replay(
        &mut self,
        key: &str,
        room: &str,
        nonce: &str,
        expires: u64,
        device: &Device,
        pin: &Pin,
    ) -> Result<(), Failure> {
        if let Some(cached) = self.ledger.cached.get(key) {
            if cached.pin != *pin || cached.room != room || cached.expires != expires {
                return Err(Failure::InvalidResponse);
            }
            match cached.state {
                CacheState::Tombstone | CacheState::Deleting => return Ok(()),
                CacheState::Preparing => {}
                CacheState::Ready => return self.replay(key, device, pin).await,
            }
        } else {
            if self.ledger.cached.len() >= 512
                || self.ledger.cached.values().map(|c| c.bytes).sum::<usize>() + USER_LIMIT
                    > CACHE_LIMIT
            {
                return Err(Failure::TooLarge);
            }
            // Reserve quota and a recoverable index before exporting payloads.
            self.ledger.cached.insert(
                key.to_owned(),
                Cached {
                    room: room.to_owned(),
                    pin: pin.clone(),
                    nonce: nonce.to_owned(),
                    expires,
                    state: CacheState::Preparing,
                    bytes: USER_LIMIT,
                    response: String::new(),
                },
            );
            self.save().await?;
        }
        let id = RoomId::parse(room).map_err(|_| Failure::InvalidResponse)?;
        let bundle = self
            .client
            .encryption()
            .shared_history_bundle(&id, 4096, USER_LIMIT - 512 * 1024)
            .await
            .map_err(|_| Failure::Storage)?
            .ok_or(Failure::TooLarge)?;
        let mut bundles = Vec::new();
        let mut current = RoomKeyBundle::default();
        let mut size = 0;
        for key in bundle.room_keys {
            let bytes = serde_json::to_vec(&key)
                .map_err(|_| Failure::InvalidResponse)?
                .len();
            if bytes + 2048 > MESSAGE_LIMIT {
                return Err(Failure::TooLarge);
            }
            if size + bytes + 2048 > MESSAGE_LIMIT {
                bundles.push(current);
                current = RoomKeyBundle::default();
                size = 0;
            }
            size += bytes + 1;
            current.room_keys.push(key);
        }
        bundles.push(current);
        if bundles.len() > usize::from(MAX_CHUNKS) {
            return Err(Failure::TooLarge);
        }
        let response = random_id()?;
        let count = bundles.len() as u16;
        let wires: Vec<String> = bundles
            .into_iter()
            .enumerate()
            .map(|(index, bundle)| {
                serde_json::to_string(&Wire::Chunk {
                    room: room.to_owned(),
                    nonce: nonce.to_owned(),
                    response: response.clone(),
                    count,
                    index: index as u16,
                    bundle,
                })
            })
            .collect::<Result<_, _>>()
            .map_err(|_| Failure::InvalidResponse)?;
        let payload = serde_json::to_vec(&wires).map_err(|_| Failure::Storage)?;
        if payload.len() > USER_LIMIT || wires.iter().any(|w| w.len() > MESSAGE_LIMIT) {
            return Err(Failure::TooLarge);
        }
        let cached = &self.ledger.cached[key];
        if self
            .store_value(&cached.storage_key(), &payload)
            .await
            .is_err()
        {
            self.poisoned.store(true, Ordering::Release);
            return Err(Failure::Storage);
        }
        let cached = self.ledger.cached.get_mut(key).ok_or(Failure::Storage)?;
        cached.state = CacheState::Ready;
        cached.response = response;
        cached.bytes = payload.len();
        self.save().await?;
        self.replay(key, device, pin).await
    }

    async fn replay(&mut self, key: &str, device: &Device, pin: &Pin) -> Result<(), Failure> {
        let cached = self
            .ledger
            .cached
            .get(key)
            .cloned()
            .ok_or(Failure::Storage)?;
        let result = self
            .client
            .encryption()
            .application_crypto_data(&cached.storage_key())
            .await;
        let payload = match result {
            Ok(Some(payload)) if payload.len() <= USER_LIMIT && payload.len() == cached.bytes => {
                payload
            }
            _ => {
                self.poisoned.store(true, Ordering::Release);
                return Err(Failure::Storage);
            }
        };
        let wires: Vec<String> = match serde_json::from_slice(&payload) {
            Ok(wires) => wires,
            Err(_) => {
                self.poisoned.store(true, Ordering::Release);
                return Err(Failure::Storage);
            }
        };
        if wires.is_empty()
            || wires.len() > usize::from(MAX_CHUNKS)
            || wires.iter().any(|w| w.len() > MESSAGE_LIMIT)
        {
            self.poisoned.store(true, Ordering::Release);
            return Err(Failure::Storage);
        }
        self.diagnostics.replayed_responses = self.diagnostics.replayed_responses.saturating_add(1);
        for raw in wires {
            if cached.expires <= now() {
                return Err(Failure::InvalidResponse);
            }
            self.client
                .encryption()
                .request_user_identity(device.user_id())
                .await
                .map_err(|_| Failure::Network)?;
            let current = self
                .client
                .encryption()
                .get_device(device.user_id(), device.device_id())
                .await
                .map_err(|_| Failure::Network)?
                .ok_or(Failure::VerificationRequired)?;
            if self.pin(&current)? != *pin {
                return Err(Failure::VerificationRequired);
            }
            if now()
                .checked_sub(self.ledger.egress_started)
                .is_some_and(|elapsed| elapsed >= 60)
            {
                self.ledger.egress_started = now();
                self.ledger.egress_bytes = 0;
            }
            if self.ledger.egress_bytes + raw.len() > CACHE_LIMIT {
                return Err(Failure::RateLimited);
            }
            self.ledger.egress_bytes += raw.len();
            self.save().await?;
            #[cfg(any(test, feature = "test-hooks"))]
            if let Some((reached, resume)) = self.disclosure_pause.take() {
                let _ = reached.send(());
                let _ = resume.await;
            }
            // Identity refresh and quota persistence can await. Refresh the
            // authoritative room policy after both, immediately before send.
            let facts = self.facts(&cached.room).await?;
            if !facts.joined.contains(&pin.user) {
                return Err(Failure::MembershipChanged);
            }
            let current = self.pinned_device(pin).await?;
            self.local_disclosure_allowed(&cached.room)?;
            self.send_raw(&[current], raw).await?;
        }
        Ok(())
    }
}
