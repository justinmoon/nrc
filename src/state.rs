use crate::config::get_default_relays;
use crate::types::AppState;
use crate::Nrc;
use anyhow::Result;
use nostr_sdk::prelude::*;
use openmls::group::GroupId;
use std::time::Duration;

impl Nrc {
    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.state = AppState::Initializing;

        self.publish_key_package().await?;

        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }

    pub async fn initialize_with_display_name(&mut self, display_name: String) -> Result<()> {
        self.state = AppState::Initializing;

        // Publish profile with display name
        self.publish_profile(display_name).await?;

        // Then publish key package
        self.publish_key_package().await?;

        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }

    pub async fn initialize_with_nsec(&mut self, nsec: String) -> Result<()> {
        let keys = Keys::parse(&nsec)?;
        self.keys = keys;
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Initialize with a display name and password (for new users)
    pub async fn initialize_with_display_name_and_password(
        &mut self,
        display_name: String,
        password: String,
    ) -> Result<()> {
        self.state = AppState::Initializing;

        // Save the keys encrypted with the password (npub is derived from keys)
        self.key_storage.save_encrypted(&self.keys, &password)?;

        // Publish profile with display name
        self.publish_profile(display_name).await?;

        // Then publish key package
        self.publish_key_package().await?;

        // Load groups and set up subscriptions
        self.initialize_groups().await?;

        Ok(())
    }

    /// Initialize with nsec and password (for importing existing keys)
    pub async fn initialize_with_nsec_and_password(
        &mut self,
        nsec: String,
        password: String,
    ) -> Result<()> {
        let keys = Keys::parse(&nsec)?;
        self.keys = keys;

        // Save the imported keys encrypted with the password (npub is derived from keys)
        self.key_storage.save_encrypted(&self.keys, &password)?;

        // Recreate client with new keys
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Initialize with password (for returning users)
    pub async fn initialize_with_password(&mut self, password: String) -> Result<()> {
        // Load and decrypt the stored keys
        let keys = self.key_storage.load_encrypted(&password)?;
        self.keys = keys;

        let npub = self.keys.public_key().to_bech32()?;
        log::info!("Loaded keys for npub: {npub}");

        // Recreate client with loaded keys
        self.client = Client::builder().signer(self.keys.clone()).build();

        for &relay in get_default_relays() {
            if let Err(e) = self.client.add_relay(relay).await {
                log::warn!("Failed to add relay {relay}: {e}");
            }
        }

        self.client.connect().await;
        tokio::time::sleep(Duration::from_secs(2)).await;

        self.initialize().await
    }

    /// Helper to initialize groups after keys are set up
    pub async fn initialize_groups(&mut self) -> Result<()> {
        let groups = self.storage.get_groups()?;
        let group_ids: Vec<GroupId> = groups.iter().map(|g| g.mls_group_id.clone()).collect();

        // Store groups in our HashMap for later use and subscribe to messages
        for group in groups {
            self.groups
                .insert(group.mls_group_id.clone(), group.clone());

            // Subscribe to messages for this group
            let h_tag_value = hex::encode(group.nostr_group_id);
            let filter = Filter::new()
                .kind(Kind::from(445u16))
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), h_tag_value)
                .limit(100);
            let _ = self.client.subscribe(filter, None).await;
        }

        self.state = AppState::Ready {
            key_package_published: true,
            groups: group_ids,
        };
        Ok(())
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
        self.flash_message = None;
    }

    pub fn dismiss_help(&mut self) {
        self.show_help = false;
        self.help_explicitly_requested = false;
    }
}
