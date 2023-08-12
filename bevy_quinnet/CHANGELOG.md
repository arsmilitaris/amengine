# Changelog

## Version 0.5.0 (2023-07-11)

- Update plugins, tests & examples to use Bevy 0.11
- Update quinn & rustls
- Update other dependencies: tokio, bytes, rcgen
- [example:breakout] Reduce the collision sound volume

## Version 0.4.0 (2023-03-07)

- Update Bevy to 0.10
- [client]: Add missing `try_send_message_on` and `try_send_payload_on` in `connection`
- Internal improvements to channels and server's `send_group_message`
- `AsyncRuntime` is now pub
- [client & server] In order to have more control over plugin initialization and only do the strict necessary, which is registering systems and events in the Bevy schedule, `.add_plugin(QuinnetClient/ServerPlugin { initialize_later: true })` can now be used, which will not create the Client/Server `Resource` immediately.
  - A command such as `commands.init_resource::<Client/Server>();` can be used later on when needed.
  - Client & server plugins systems are scheduled to only run if their respective resource exists.
  - Their respective `Resource` can be removed through a command at runtime.
- [client]: Fix IPv6 handling.
  - Remove `ConnectionConfiguration::new`
  - Add `ConnectionConfiguration::from_strings`, `ConnectionConfiguration::from_ips` and `ConnectionConfiguration::from_addrs`
- [server]: Fix IPv6 handling.
  - Rename `ServerConfigurationData` to `ServerConfiguration`.
  - Remove `ServerConfiguration::new`
  - Add `ServerConfiguration::from_string`, `ServerConfiguration::from_ip` and `ServerConfiguration::from_addr`
  - Add `server_hostname` to `CertificateRetrievalMode::GenerateSelfSigned` and `CertificateRetrievalMode::LoadFromFileOrGenerateSelfSigned`

## Version 0.3.0 (2023-01-20)

### Added

- [client & server] Add `OrderedReliable`, `UnorderedReliable` and `Unreliable` send channels. The existing API uses default channels, additional channel can be opened/closed with `open_channel`/`close_channel` and used with derivatives of `send_message_on`.
- [client & server] Now also receive `Unreliable` messages
- [client & server] Add a stats() function on a connection to retrieve statistics (thanks to [Andrewvy](https://github.com/andrewvy), PR [#4](https://github.com/Henauxg/bevy_quinnet/pull/4))
- [server] Add a clients() function to retrieve all the connected Client ids
- [tests] Add tests for channels, and move tests to cargo integration test directory

### Changed

- [server] `receive_message` and `receive_payload` functions are now `receive_message_from` and `receive_payload_from`, taking a ClientId as parameter (clients messages are now stored on separate queues to prevent a client from filling the shared queue)
- [client] Expose `ConnectionId` as `pub` in `ConnectionEvent` and `ConnectionLostEvent ` (thanks to [Zheilbron](https://github.com/zheilbron), PR [#5](https://github.com/Henauxg/bevy_quinnet/pull/5))
- [example:breakout] Make use of Unreliable and UnorderedReliable channels
- Updated dependencies

### Fixed
- [client & server] Enhancement on disconnection behaviours, existing outgoing messages are now flushed (thanks to [Zheilbron](https://github.com/zheilbron), PR [#6](https://github.com/Henauxg/bevy_quinnet/pull/6))
- [client] Do not fail in store_known_hosts_to_file if the path has no prefix
- [server] Do not fail in write_certs_to_files if the cert and key files have non-existing parent directories, create them instead

## Version 0.2.0 (2022-11-18)

### Added

- New events: ConnectionEvent and ConnectionLostEvent on both client & server
- Implemented Trust on First Use authentication scheme on the client
  - Added CertificateVerificationMode for client : SkipVerification, SignedByCertificateAuthority and TrustOnFirstUse
  - New client events for the certificate verification: CertInteractionEvent, CertTrustUpdateEvent and CertConnectionAbortEvent
- Added new ways to handle server certificates with CertificateRetrievalMode: GenerateSelfSigned, LoadFromFile & LoadFromFileOrGenerateSelfSigned
- Client can now have multiple connections simultaneously to multiple servers
- Server now returns the generated/loaded certificate
- It is now possible to host a server locally on a client
  - New example: Bevy breakout demo as a 2 players versus game, hosted from a client
- New open_connection & close_connection methods on the client
- New start_endpoint and close_endpoint on the server
- New is_listening method on the server
- New "try" methods that log the errors
- Added tests
- Added documentation

### Changed

- Client & Server configurations now taken as parameter at connection/listen time rather than from a resource. (User can still store it inside a resource)
- Raised DEFAULT_INTERNAL_MESSAGE_CHANNEL_SIZE in client to 100
- Use thiserror internally
- Update Quinn to 0.9
- Update Bevy to 0.9 (by [Lemonzy](https://github.com/Lemonzyy))
- Updated all dependencies (minors)
- Moved chat demo to its own directory

## Version 0.1.0 (2022-10-25)

Initial release