#[cfg(test)]
mod service_tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use warpui::r#async::executor::Background;

    use crate::{JsonRpcService, Transport};

    #[derive(Debug, Clone)]
    struct MockTransport {
        sent_messages: Arc<Mutex<Vec<String>>>,
        incoming_receiver: async_channel::Receiver<String>,
        incoming_sender: async_channel::Sender<String>,
    }

    impl MockTransport {
        fn new() -> Self {
            let (sender, receiver) = async_channel::unbounded();
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
                incoming_receiver: receiver,
                incoming_sender: sender,
            }
        }

        fn send_incoming_message(&self, message: &str) {
            self.incoming_sender
                .try_send(message.to_string())
                .expect("Failed to send message");
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn read(&self) -> anyhow::Result<String> {
            match self.incoming_receiver.recv().await {
                Ok(message) => Ok(message),
                Err(_) => Ok(String::new()), // Channel closed, simulate EOF
            }
        }

        async fn write(&self, message: &str) -> anyhow::Result<()> {
            self.sent_messages.lock().unwrap().push(message.to_string());
            Ok(())
        }

        async fn shutdown(&self, _timeout: std::time::Duration) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_json_rpc_service_creation() {
        let mock_transport = MockTransport::new();
        let _service =
            JsonRpcService::new(Box::new(mock_transport), Arc::new(Background::default()), 0);
        // Test passes if service creation doesn't panic
    }

    #[test]
    fn test_mock_transport_message_storage() {
        let mock_transport = MockTransport::new();
        mock_transport.send_incoming_message("test message");

        // The message should be in the channel, we can verify by trying to receive it
        futures::executor::block_on(async {
            let message = mock_transport.incoming_receiver.recv().await.unwrap();
            assert_eq!(message, "test message");
        });
    }
}
