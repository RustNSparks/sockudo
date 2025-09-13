use crate::adapter::horizontal_adapter_helpers::{MockConfig, MockTransport};
use sockudo::adapter::horizontal_adapter::{RequestType, ResponseBody};
use sockudo::adapter::horizontal_adapter_base::HorizontalAdapterBase;
use std::collections::{HashMap, HashSet};

#[tokio::test]
async fn test_request_response_aggregation_logic() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Create a test request that would be sent to other nodes
    let request_id = "test-request-123";
    let node_id = adapter.node_id.clone();

    // Simulate multiple responses from different nodes
    let responses = vec![
        // Response 1: Node has 2 sockets in channel
        ResponseBody {
            request_id: request_id.to_string(),
            node_id: "node-1".to_string(),
            app_id: "test-app".to_string(),
            members: HashMap::new(),
            channels_with_sockets_count: HashMap::new(),
            socket_ids: vec!["socket-1".to_string(), "socket-2".to_string()],
            sockets_count: 2,
            exists: true,
            channels: HashSet::new(),
            members_count: 0,
        },
        // Response 2: Node has 1 socket in channel
        ResponseBody {
            request_id: request_id.to_string(),
            node_id: "node-2".to_string(),
            app_id: "test-app".to_string(),
            members: HashMap::new(),
            channels_with_sockets_count: HashMap::new(),
            socket_ids: vec!["socket-3".to_string()],
            sockets_count: 1,
            exists: true,
            channels: HashSet::new(),
            members_count: 0,
        },
    ];

    // Test the aggregation logic
    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id,
            "test-app".to_string(),
            &RequestType::ChannelSockets,
            responses,
        )
    };

    // Verify aggregation results
    assert_eq!(combined_response.request_id, request_id);
    assert_eq!(combined_response.app_id, "test-app");
    assert_eq!(combined_response.socket_ids.len(), 3); // Combined from both nodes
    assert_eq!(combined_response.sockets_count, 3); // Sum of both nodes
    // Note: exists field is not aggregated for ChannelSockets request type

    // Verify all socket IDs are present
    let socket_ids: HashSet<String> = combined_response.socket_ids.into_iter().collect();
    assert!(socket_ids.contains("socket-1"));
    assert!(socket_ids.contains("socket-2"));
    assert!(socket_ids.contains("socket-3"));
}

#[tokio::test]
async fn test_channel_members_aggregation_deduplication() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let request_id = "members-test";
    let node_id = adapter.node_id.clone();

    // Create responses with overlapping members (same user on different nodes)
    let mut responses = Vec::new();

    // Node 1: Has user-1 and user-2
    let mut members1 = HashMap::new();
    members1.insert(
        "user-1".to_string(),
        sockudo::channel::PresenceMemberInfo {
            user_id: "user-1".to_string(),
            user_info: Some(serde_json::json!({"name": "Alice", "node": "node-1"})),
        },
    );
    members1.insert(
        "user-2".to_string(),
        sockudo::channel::PresenceMemberInfo {
            user_id: "user-2".to_string(),
            user_info: Some(serde_json::json!({"name": "Bob", "node": "node-1"})),
        },
    );

    responses.push(ResponseBody {
        request_id: request_id.to_string(),
        node_id: "node-1".to_string(),
        app_id: "test-app".to_string(),
        members: members1,
        channels_with_sockets_count: HashMap::new(),
        socket_ids: Vec::new(),
        sockets_count: 0,
        exists: false,
        channels: HashSet::new(),
        members_count: 2,
    });

    // Node 2: Has user-1 (duplicate) and user-3 (unique)
    let mut members2 = HashMap::new();
    members2.insert(
        "user-1".to_string(),
        sockudo::channel::PresenceMemberInfo {
            user_id: "user-1".to_string(),
            user_info: Some(serde_json::json!({"name": "Alice", "node": "node-2"})), // Different data
        },
    );
    members2.insert(
        "user-3".to_string(),
        sockudo::channel::PresenceMemberInfo {
            user_id: "user-3".to_string(),
            user_info: Some(serde_json::json!({"name": "Carol", "node": "node-2"})),
        },
    );

    responses.push(ResponseBody {
        request_id: request_id.to_string(),
        node_id: "node-2".to_string(),
        app_id: "test-app".to_string(),
        members: members2,
        channels_with_sockets_count: HashMap::new(),
        socket_ids: Vec::new(),
        sockets_count: 0,
        exists: false,
        channels: HashSet::new(),
        members_count: 2,
    });

    // Test aggregation
    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id,
            "test-app".to_string(),
            &RequestType::ChannelMembers,
            responses,
        )
    };

    // Verify deduplication occurred but all unique users are present
    assert_eq!(combined_response.members.len(), 3); // user-1, user-2, user-3
    assert!(combined_response.members.contains_key("user-1"));
    assert!(combined_response.members.contains_key("user-2"));
    assert!(combined_response.members.contains_key("user-3"));

    // For duplicate user-1, the aggregation should keep one of the entries
    // (behavior depends on implementation - either first or last wins)
    let user1_data = &combined_response.members["user-1"];
    assert_eq!(user1_data.user_id, "user-1");
    assert!(user1_data.user_info.is_some());
}

#[tokio::test]
async fn test_channels_with_socket_count_aggregation() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let request_id = "channels-count-test";
    let node_id = adapter.node_id.clone();

    let mut responses = Vec::new();

    // Node 1: Has channels A(2 sockets), B(1 socket)
    let mut channels1 = HashMap::new();
    channels1.insert("channel-A".to_string(), 2);
    channels1.insert("channel-B".to_string(), 1);

    responses.push(ResponseBody {
        request_id: request_id.to_string(),
        node_id: "node-1".to_string(),
        app_id: "test-app".to_string(),
        members: HashMap::new(),
        channels_with_sockets_count: channels1,
        socket_ids: Vec::new(),
        sockets_count: 0,
        exists: false,
        channels: HashSet::new(),
        members_count: 0,
    });

    // Node 2: Has channels A(3 sockets), C(1 socket)
    let mut channels2 = HashMap::new();
    channels2.insert("channel-A".to_string(), 3); // Overlaps with node-1
    channels2.insert("channel-C".to_string(), 1); // Unique to node-2

    responses.push(ResponseBody {
        request_id: request_id.to_string(),
        node_id: "node-2".to_string(),
        app_id: "test-app".to_string(),
        members: HashMap::new(),
        channels_with_sockets_count: channels2,
        socket_ids: Vec::new(),
        sockets_count: 0,
        exists: false,
        channels: HashSet::new(),
        members_count: 0,
    });

    // Test aggregation
    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id,
            "test-app".to_string(),
            &RequestType::ChannelsWithSocketsCount,
            responses,
        )
    };

    // Verify counts are properly summed
    assert_eq!(combined_response.channels_with_sockets_count.len(), 3);
    assert_eq!(
        combined_response.channels_with_sockets_count["channel-A"],
        5
    ); // 2 + 3
    assert_eq!(
        combined_response.channels_with_sockets_count["channel-B"],
        1
    ); // Only from node-1
    assert_eq!(
        combined_response.channels_with_sockets_count["channel-C"],
        1
    ); // Only from node-2
}

#[tokio::test]
async fn test_exists_flag_aggregation_logic() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let request_id = "exists-test";
    let node_id = adapter.node_id.clone();

    // Test case 1: No nodes return exists=true
    let mut responses = Vec::new();
    for i in 0..3 {
        responses.push(ResponseBody {
            request_id: request_id.to_string(),
            node_id: format!("node-{}", i),
            app_id: "test-app".to_string(),
            members: HashMap::new(),
            channels_with_sockets_count: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            exists: false, // All return false
            channels: HashSet::new(),
            members_count: 0,
        });
    }

    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id.clone(),
            "test-app".to_string(),
            &RequestType::SocketExistsInChannel,
            responses,
        )
    };

    assert!(
        !combined_response.exists,
        "Should be false when no node returns true"
    );

    // Test case 2: One node returns exists=true
    let responses = vec![
        ResponseBody {
            request_id: request_id.to_string(),
            node_id: "node-0".to_string(),
            app_id: "test-app".to_string(),
            members: HashMap::new(),
            channels_with_sockets_count: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            exists: false,
            channels: HashSet::new(),
            members_count: 0,
        },
        ResponseBody {
            request_id: request_id.to_string(),
            node_id: "node-1".to_string(),
            app_id: "test-app".to_string(),
            members: HashMap::new(),
            channels_with_sockets_count: HashMap::new(),
            socket_ids: Vec::new(),
            sockets_count: 0,
            exists: true, // One returns true
            channels: HashSet::new(),
            members_count: 0,
        },
    ];

    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id,
            "test-app".to_string(),
            &RequestType::SocketExistsInChannel,
            responses,
        )
    };

    assert!(
        combined_response.exists,
        "Should be true when any node returns true"
    );
}

#[tokio::test]
async fn test_request_with_no_other_nodes_returns_immediately() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    adapter.start_listeners().await.unwrap();

    // When there are no other nodes, request should return immediately
    let start_time = std::time::Instant::now();
    let result = adapter
        .send_request(
            "single-node-app",
            RequestType::ChannelSockets,
            Some("test-channel"),
            None,
            None,
        )
        .await;

    let elapsed = start_time.elapsed();

    // Should complete immediately without waiting for timeout
    assert!(result.is_ok(), "Request should complete for single node");
    // Should return very quickly (less than 50ms) since no waiting is needed
    assert!(
        elapsed.as_millis() < 50,
        "Should return immediately when no other nodes exist"
    );

    // Response should indicate no results from other nodes
    let response = result.unwrap();
    assert_eq!(response.app_id, "single-node-app");
    assert!(response.socket_ids.is_empty());
    assert_eq!(response.sockets_count, 0);
}

#[tokio::test]
async fn test_partial_response_handling_timeout_behavior() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    // Set very short timeout for testing
    {
        let mut horizontal = adapter.horizontal.lock().await;
        horizontal.requests_timeout = 100; // 100ms timeout
    }

    // Add multiple nodes to simulate expecting multiple responses
    adapter.transport.add_node().await;
    adapter.transport.add_node().await; // Now we have 3 total nodes

    let start_time = std::time::Instant::now();

    // This request will timeout since there are no real nodes to respond
    let result = adapter
        .send_request(
            "timeout-test-app",
            RequestType::ChannelSockets,
            Some("test-channel"),
            None,
            None,
        )
        .await;

    let elapsed = start_time.elapsed();

    // Should complete due to timeout, not hang indefinitely
    assert!(result.is_ok(), "Request should complete even on timeout");
    // Only verify minimum timeout was respected - no upper bound as that's system-dependent
    assert!(
        elapsed.as_millis() >= 100,
        "Should wait at least the timeout duration"
    );

    // Response should indicate no results from remote nodes
    let response = result.unwrap();
    assert_eq!(response.app_id, "timeout-test-app");
    // In timeout scenarios, we get an aggregated response with just local data
}

#[tokio::test]
async fn test_empty_response_aggregation() {
    let config = MockConfig::default();
    let adapter = HorizontalAdapterBase::<MockTransport>::new(config)
        .await
        .unwrap();

    let request_id = "empty-test";
    let node_id = adapter.node_id.clone();

    // Test aggregation with completely empty responses
    let responses = Vec::new(); // No responses at all

    let combined_response = {
        let horizontal = adapter.horizontal.lock().await;
        horizontal.aggregate_responses(
            request_id.to_string(),
            node_id,
            "test-app".to_string(),
            &RequestType::ChannelSockets,
            responses,
        )
    };

    // Should return valid empty response
    assert_eq!(combined_response.request_id, request_id);
    assert_eq!(combined_response.app_id, "test-app");
    assert!(combined_response.socket_ids.is_empty());
    assert_eq!(combined_response.sockets_count, 0);
    assert!(!combined_response.exists);
    assert!(combined_response.members.is_empty());
    assert!(combined_response.channels_with_sockets_count.is_empty());
}
