## Default Permission

Status-only default permissions for the BLE plugin.

#### This default permission set includes the following:

- `allow-get-state`
- `allow-get-capabilities`
- `allow-check-permissions`
- `allow-close`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`gattify:advertise`

</td>
<td>

Start and stop BLE advertising.

</td>
</tr>

<tr>
<td>

`gattify:allow-cancel`

</td>
<td>

Enables the cancel command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-cancel`

</td>
<td>

Denies the cancel command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-check-permissions`

</td>
<td>

Enables the check_permissions command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-check-permissions`

</td>
<td>

Denies the check_permissions command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-close`

</td>
<td>

Enables the close command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-close`

</td>
<td>

Denies the close command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-close-endpoint`

</td>
<td>

Enables the close_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-close-endpoint`

</td>
<td>

Denies the close_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-close-peer`

</td>
<td>

Enables the close_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-close-peer`

</td>
<td>

Denies the close_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-create-endpoint`

</td>
<td>

Enables the create_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-create-endpoint`

</td>
<td>

Denies the create_endpoint command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-dial-peer`

</td>
<td>

Enables the dial_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-dial-peer`

</td>
<td>

Denies the dial_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-execute-advertise`

</td>
<td>

Enables the execute_advertise command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-execute-advertise`

</td>
<td>

Denies the execute_advertise command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-execute-connect`

</td>
<td>

Enables the execute_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-execute-connect`

</td>
<td>

Denies the execute_connect command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-execute-scan`

</td>
<td>

Enables the execute_scan command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-execute-scan`

</td>
<td>

Denies the execute_scan command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-execute-server`

</td>
<td>

Enables the execute_server command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-execute-server`

</td>
<td>

Denies the execute_server command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-get-capabilities`

</td>
<td>

Enables the get_capabilities command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-get-capabilities`

</td>
<td>

Denies the get_capabilities command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-get-state`

</td>
<td>

Enables the get_state command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-get-state`

</td>
<td>

Denies the get_state command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-request-advertise-permission`

</td>
<td>

Enables the request_advertise_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-request-advertise-permission`

</td>
<td>

Denies the request_advertise_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-request-connect-permission`

</td>
<td>

Enables the request_connect_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-request-connect-permission`

</td>
<td>

Denies the request_connect_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-request-scan-permission`

</td>
<td>

Enables the request_scan_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-request-scan-permission`

</td>
<td>

Denies the request_scan_permission command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:allow-send-peer`

</td>
<td>

Enables the send_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:deny-send-peer`

</td>
<td>

Denies the send_peer command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`gattify:connect`

</td>
<td>

Connect and perform GATT client procedures.

</td>
</tr>

<tr>
<td>

`gattify:peer`

</td>
<td>

Create endpoints, connect peers, exchange messages and close peer resources.

</td>
</tr>

<tr>
<td>

`gattify:scan`

</td>
<td>

Start and stop scoped BLE scans.

</td>
</tr>

<tr>
<td>

`gattify:server`

</td>
<td>

Register and manage a local GATT server.

</td>
</tr>

<tr>
<td>

`gattify:status`

</td>
<td>

Read adapter state, capabilities and permission state.

</td>
</tr>
</table>
