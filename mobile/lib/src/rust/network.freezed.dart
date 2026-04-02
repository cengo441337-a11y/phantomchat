// coverage:ignore-file
// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'network.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

T _$identity<T>(T value) => value;

final _privateConstructorUsedError = UnsupportedError(
    'It seems like you constructed your class using `MyClass._()`. This constructor is only meant to be used by freezed and you are not supposed to need it nor use it.\nPlease check the documentation here for more information: https://github.com/rrousselGit/freezed#adding-getters-and-methods-to-our-models');

/// @nodoc
mixin _$NetworkEvent {
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) =>
      throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) =>
      throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) =>
      throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) =>
      throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) =>
      throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) =>
      throw _privateConstructorUsedError;
}

/// @nodoc
abstract class $NetworkEventCopyWith<$Res> {
  factory $NetworkEventCopyWith(
          NetworkEvent value, $Res Function(NetworkEvent) then) =
      _$NetworkEventCopyWithImpl<$Res, NetworkEvent>;
}

/// @nodoc
class _$NetworkEventCopyWithImpl<$Res, $Val extends NetworkEvent>
    implements $NetworkEventCopyWith<$Res> {
  _$NetworkEventCopyWithImpl(this._value, this._then);

  // ignore: unused_field
  final $Val _value;
  // ignore: unused_field
  final $Res Function($Val) _then;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
}

/// @nodoc
abstract class _$$NetworkEvent_NodeStartedImplCopyWith<$Res> {
  factory _$$NetworkEvent_NodeStartedImplCopyWith(
          _$NetworkEvent_NodeStartedImpl value,
          $Res Function(_$NetworkEvent_NodeStartedImpl) then) =
      __$$NetworkEvent_NodeStartedImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String peerId});
}

/// @nodoc
class __$$NetworkEvent_NodeStartedImplCopyWithImpl<$Res>
    extends _$NetworkEventCopyWithImpl<$Res, _$NetworkEvent_NodeStartedImpl>
    implements _$$NetworkEvent_NodeStartedImplCopyWith<$Res> {
  __$$NetworkEvent_NodeStartedImplCopyWithImpl(
      _$NetworkEvent_NodeStartedImpl _value,
      $Res Function(_$NetworkEvent_NodeStartedImpl) _then)
      : super(_value, _then);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? peerId = null,
  }) {
    return _then(_$NetworkEvent_NodeStartedImpl(
      peerId: null == peerId
          ? _value.peerId
          : peerId // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class _$NetworkEvent_NodeStartedImpl extends NetworkEvent_NodeStarted {
  const _$NetworkEvent_NodeStartedImpl({required this.peerId}) : super._();

  @override
  final String peerId;

  @override
  String toString() {
    return 'NetworkEvent.nodeStarted(peerId: $peerId)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$NetworkEvent_NodeStartedImpl &&
            (identical(other.peerId, peerId) || other.peerId == peerId));
  }

  @override
  int get hashCode => Object.hash(runtimeType, peerId);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$NetworkEvent_NodeStartedImplCopyWith<_$NetworkEvent_NodeStartedImpl>
      get copyWith => __$$NetworkEvent_NodeStartedImplCopyWithImpl<
          _$NetworkEvent_NodeStartedImpl>(this, _$identity);

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) {
    return nodeStarted(peerId);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) {
    return nodeStarted?.call(peerId);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) {
    if (nodeStarted != null) {
      return nodeStarted(peerId);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) {
    return nodeStarted(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) {
    return nodeStarted?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) {
    if (nodeStarted != null) {
      return nodeStarted(this);
    }
    return orElse();
  }
}

abstract class NetworkEvent_NodeStarted extends NetworkEvent {
  const factory NetworkEvent_NodeStarted({required final String peerId}) =
      _$NetworkEvent_NodeStartedImpl;
  const NetworkEvent_NodeStarted._() : super._();

  String get peerId;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$NetworkEvent_NodeStartedImplCopyWith<_$NetworkEvent_NodeStartedImpl>
      get copyWith => throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$NetworkEvent_PeerDiscoveredImplCopyWith<$Res> {
  factory _$$NetworkEvent_PeerDiscoveredImplCopyWith(
          _$NetworkEvent_PeerDiscoveredImpl value,
          $Res Function(_$NetworkEvent_PeerDiscoveredImpl) then) =
      __$$NetworkEvent_PeerDiscoveredImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String peerId, String? avatarCid});
}

/// @nodoc
class __$$NetworkEvent_PeerDiscoveredImplCopyWithImpl<$Res>
    extends _$NetworkEventCopyWithImpl<$Res, _$NetworkEvent_PeerDiscoveredImpl>
    implements _$$NetworkEvent_PeerDiscoveredImplCopyWith<$Res> {
  __$$NetworkEvent_PeerDiscoveredImplCopyWithImpl(
      _$NetworkEvent_PeerDiscoveredImpl _value,
      $Res Function(_$NetworkEvent_PeerDiscoveredImpl) _then)
      : super(_value, _then);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? peerId = null,
    Object? avatarCid = freezed,
  }) {
    return _then(_$NetworkEvent_PeerDiscoveredImpl(
      peerId: null == peerId
          ? _value.peerId
          : peerId // ignore: cast_nullable_to_non_nullable
              as String,
      avatarCid: freezed == avatarCid
          ? _value.avatarCid
          : avatarCid // ignore: cast_nullable_to_non_nullable
              as String?,
    ));
  }
}

/// @nodoc

class _$NetworkEvent_PeerDiscoveredImpl extends NetworkEvent_PeerDiscovered {
  const _$NetworkEvent_PeerDiscoveredImpl(
      {required this.peerId, this.avatarCid})
      : super._();

  @override
  final String peerId;
  @override
  final String? avatarCid;

  @override
  String toString() {
    return 'NetworkEvent.peerDiscovered(peerId: $peerId, avatarCid: $avatarCid)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$NetworkEvent_PeerDiscoveredImpl &&
            (identical(other.peerId, peerId) || other.peerId == peerId) &&
            (identical(other.avatarCid, avatarCid) ||
                other.avatarCid == avatarCid));
  }

  @override
  int get hashCode => Object.hash(runtimeType, peerId, avatarCid);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$NetworkEvent_PeerDiscoveredImplCopyWith<_$NetworkEvent_PeerDiscoveredImpl>
      get copyWith => __$$NetworkEvent_PeerDiscoveredImplCopyWithImpl<
          _$NetworkEvent_PeerDiscoveredImpl>(this, _$identity);

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) {
    return peerDiscovered(peerId, avatarCid);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) {
    return peerDiscovered?.call(peerId, avatarCid);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) {
    if (peerDiscovered != null) {
      return peerDiscovered(peerId, avatarCid);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) {
    return peerDiscovered(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) {
    return peerDiscovered?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) {
    if (peerDiscovered != null) {
      return peerDiscovered(this);
    }
    return orElse();
  }
}

abstract class NetworkEvent_PeerDiscovered extends NetworkEvent {
  const factory NetworkEvent_PeerDiscovered(
      {required final String peerId,
      final String? avatarCid}) = _$NetworkEvent_PeerDiscoveredImpl;
  const NetworkEvent_PeerDiscovered._() : super._();

  String get peerId;
  String? get avatarCid;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$NetworkEvent_PeerDiscoveredImplCopyWith<_$NetworkEvent_PeerDiscoveredImpl>
      get copyWith => throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$NetworkEvent_MessageReceivedImplCopyWith<$Res> {
  factory _$$NetworkEvent_MessageReceivedImplCopyWith(
          _$NetworkEvent_MessageReceivedImpl value,
          $Res Function(_$NetworkEvent_MessageReceivedImpl) then) =
      __$$NetworkEvent_MessageReceivedImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String from, String message});
}

/// @nodoc
class __$$NetworkEvent_MessageReceivedImplCopyWithImpl<$Res>
    extends _$NetworkEventCopyWithImpl<$Res, _$NetworkEvent_MessageReceivedImpl>
    implements _$$NetworkEvent_MessageReceivedImplCopyWith<$Res> {
  __$$NetworkEvent_MessageReceivedImplCopyWithImpl(
      _$NetworkEvent_MessageReceivedImpl _value,
      $Res Function(_$NetworkEvent_MessageReceivedImpl) _then)
      : super(_value, _then);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? from = null,
    Object? message = null,
  }) {
    return _then(_$NetworkEvent_MessageReceivedImpl(
      from: null == from
          ? _value.from
          : from // ignore: cast_nullable_to_non_nullable
              as String,
      message: null == message
          ? _value.message
          : message // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class _$NetworkEvent_MessageReceivedImpl extends NetworkEvent_MessageReceived {
  const _$NetworkEvent_MessageReceivedImpl(
      {required this.from, required this.message})
      : super._();

  @override
  final String from;
  @override
  final String message;

  @override
  String toString() {
    return 'NetworkEvent.messageReceived(from: $from, message: $message)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$NetworkEvent_MessageReceivedImpl &&
            (identical(other.from, from) || other.from == from) &&
            (identical(other.message, message) || other.message == message));
  }

  @override
  int get hashCode => Object.hash(runtimeType, from, message);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$NetworkEvent_MessageReceivedImplCopyWith<
          _$NetworkEvent_MessageReceivedImpl>
      get copyWith => __$$NetworkEvent_MessageReceivedImplCopyWithImpl<
          _$NetworkEvent_MessageReceivedImpl>(this, _$identity);

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) {
    return messageReceived(from, message);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) {
    return messageReceived?.call(from, message);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) {
    if (messageReceived != null) {
      return messageReceived(from, message);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) {
    return messageReceived(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) {
    return messageReceived?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) {
    if (messageReceived != null) {
      return messageReceived(this);
    }
    return orElse();
  }
}

abstract class NetworkEvent_MessageReceived extends NetworkEvent {
  const factory NetworkEvent_MessageReceived(
      {required final String from,
      required final String message}) = _$NetworkEvent_MessageReceivedImpl;
  const NetworkEvent_MessageReceived._() : super._();

  String get from;
  String get message;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$NetworkEvent_MessageReceivedImplCopyWith<
          _$NetworkEvent_MessageReceivedImpl>
      get copyWith => throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$NetworkEvent_GroupMessageReceivedImplCopyWith<$Res> {
  factory _$$NetworkEvent_GroupMessageReceivedImplCopyWith(
          _$NetworkEvent_GroupMessageReceivedImpl value,
          $Res Function(_$NetworkEvent_GroupMessageReceivedImpl) then) =
      __$$NetworkEvent_GroupMessageReceivedImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String groupId, String from, String message});
}

/// @nodoc
class __$$NetworkEvent_GroupMessageReceivedImplCopyWithImpl<$Res>
    extends _$NetworkEventCopyWithImpl<$Res,
        _$NetworkEvent_GroupMessageReceivedImpl>
    implements _$$NetworkEvent_GroupMessageReceivedImplCopyWith<$Res> {
  __$$NetworkEvent_GroupMessageReceivedImplCopyWithImpl(
      _$NetworkEvent_GroupMessageReceivedImpl _value,
      $Res Function(_$NetworkEvent_GroupMessageReceivedImpl) _then)
      : super(_value, _then);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? groupId = null,
    Object? from = null,
    Object? message = null,
  }) {
    return _then(_$NetworkEvent_GroupMessageReceivedImpl(
      groupId: null == groupId
          ? _value.groupId
          : groupId // ignore: cast_nullable_to_non_nullable
              as String,
      from: null == from
          ? _value.from
          : from // ignore: cast_nullable_to_non_nullable
              as String,
      message: null == message
          ? _value.message
          : message // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class _$NetworkEvent_GroupMessageReceivedImpl
    extends NetworkEvent_GroupMessageReceived {
  const _$NetworkEvent_GroupMessageReceivedImpl(
      {required this.groupId, required this.from, required this.message})
      : super._();

  @override
  final String groupId;
  @override
  final String from;
  @override
  final String message;

  @override
  String toString() {
    return 'NetworkEvent.groupMessageReceived(groupId: $groupId, from: $from, message: $message)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$NetworkEvent_GroupMessageReceivedImpl &&
            (identical(other.groupId, groupId) || other.groupId == groupId) &&
            (identical(other.from, from) || other.from == from) &&
            (identical(other.message, message) || other.message == message));
  }

  @override
  int get hashCode => Object.hash(runtimeType, groupId, from, message);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$NetworkEvent_GroupMessageReceivedImplCopyWith<
          _$NetworkEvent_GroupMessageReceivedImpl>
      get copyWith => __$$NetworkEvent_GroupMessageReceivedImplCopyWithImpl<
          _$NetworkEvent_GroupMessageReceivedImpl>(this, _$identity);

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) {
    return groupMessageReceived(groupId, from, message);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) {
    return groupMessageReceived?.call(groupId, from, message);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) {
    if (groupMessageReceived != null) {
      return groupMessageReceived(groupId, from, message);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) {
    return groupMessageReceived(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) {
    return groupMessageReceived?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) {
    if (groupMessageReceived != null) {
      return groupMessageReceived(this);
    }
    return orElse();
  }
}

abstract class NetworkEvent_GroupMessageReceived extends NetworkEvent {
  const factory NetworkEvent_GroupMessageReceived(
      {required final String groupId,
      required final String from,
      required final String message}) = _$NetworkEvent_GroupMessageReceivedImpl;
  const NetworkEvent_GroupMessageReceived._() : super._();

  String get groupId;
  String get from;
  String get message;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$NetworkEvent_GroupMessageReceivedImplCopyWith<
          _$NetworkEvent_GroupMessageReceivedImpl>
      get copyWith => throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$NetworkEvent_ErrorImplCopyWith<$Res> {
  factory _$$NetworkEvent_ErrorImplCopyWith(_$NetworkEvent_ErrorImpl value,
          $Res Function(_$NetworkEvent_ErrorImpl) then) =
      __$$NetworkEvent_ErrorImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String message});
}

/// @nodoc
class __$$NetworkEvent_ErrorImplCopyWithImpl<$Res>
    extends _$NetworkEventCopyWithImpl<$Res, _$NetworkEvent_ErrorImpl>
    implements _$$NetworkEvent_ErrorImplCopyWith<$Res> {
  __$$NetworkEvent_ErrorImplCopyWithImpl(_$NetworkEvent_ErrorImpl _value,
      $Res Function(_$NetworkEvent_ErrorImpl) _then)
      : super(_value, _then);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({
    Object? message = null,
  }) {
    return _then(_$NetworkEvent_ErrorImpl(
      message: null == message
          ? _value.message
          : message // ignore: cast_nullable_to_non_nullable
              as String,
    ));
  }
}

/// @nodoc

class _$NetworkEvent_ErrorImpl extends NetworkEvent_Error {
  const _$NetworkEvent_ErrorImpl({required this.message}) : super._();

  @override
  final String message;

  @override
  String toString() {
    return 'NetworkEvent.error(message: $message)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$NetworkEvent_ErrorImpl &&
            (identical(other.message, message) || other.message == message));
  }

  @override
  int get hashCode => Object.hash(runtimeType, message);

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$NetworkEvent_ErrorImplCopyWith<_$NetworkEvent_ErrorImpl> get copyWith =>
      __$$NetworkEvent_ErrorImplCopyWithImpl<_$NetworkEvent_ErrorImpl>(
          this, _$identity);

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function(String peerId) nodeStarted,
    required TResult Function(String peerId, String? avatarCid) peerDiscovered,
    required TResult Function(String from, String message) messageReceived,
    required TResult Function(String groupId, String from, String message)
        groupMessageReceived,
    required TResult Function(String message) error,
  }) {
    return error(message);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function(String peerId)? nodeStarted,
    TResult? Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult? Function(String from, String message)? messageReceived,
    TResult? Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult? Function(String message)? error,
  }) {
    return error?.call(message);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function(String peerId)? nodeStarted,
    TResult Function(String peerId, String? avatarCid)? peerDiscovered,
    TResult Function(String from, String message)? messageReceived,
    TResult Function(String groupId, String from, String message)?
        groupMessageReceived,
    TResult Function(String message)? error,
    required TResult orElse(),
  }) {
    if (error != null) {
      return error(message);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(NetworkEvent_NodeStarted value) nodeStarted,
    required TResult Function(NetworkEvent_PeerDiscovered value) peerDiscovered,
    required TResult Function(NetworkEvent_MessageReceived value)
        messageReceived,
    required TResult Function(NetworkEvent_GroupMessageReceived value)
        groupMessageReceived,
    required TResult Function(NetworkEvent_Error value) error,
  }) {
    return error(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult? Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult? Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult? Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult? Function(NetworkEvent_Error value)? error,
  }) {
    return error?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(NetworkEvent_NodeStarted value)? nodeStarted,
    TResult Function(NetworkEvent_PeerDiscovered value)? peerDiscovered,
    TResult Function(NetworkEvent_MessageReceived value)? messageReceived,
    TResult Function(NetworkEvent_GroupMessageReceived value)?
        groupMessageReceived,
    TResult Function(NetworkEvent_Error value)? error,
    required TResult orElse(),
  }) {
    if (error != null) {
      return error(this);
    }
    return orElse();
  }
}

abstract class NetworkEvent_Error extends NetworkEvent {
  const factory NetworkEvent_Error({required final String message}) =
      _$NetworkEvent_ErrorImpl;
  const NetworkEvent_Error._() : super._();

  String get message;

  /// Create a copy of NetworkEvent
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$NetworkEvent_ErrorImplCopyWith<_$NetworkEvent_ErrorImpl> get copyWith =>
      throw _privateConstructorUsedError;
}
