use windows::{
    core::{Error, Ref, RuntimeType, GUID},
    Devices::Bluetooth::GenericAttributeProfile::GattSession,
    Foundation::{IReference, TypedEventHandler},
    Storage::Streams::{DataReader, DataWriter, IBuffer},
};
use windows_collections::IVectorView;

use super::{
    gatt::{uuid_from_u128, uuid_to_u128},
    status::hresult_error,
};
use crate::{BleError, ErrorCode};

/// A `WinRT` event handler that passes the event arguments on. Windows calls
/// it on a thread of its own, so `on_event` only reads the arguments and
/// posts to the engine.
pub(super) fn handler<S, A>(
    mut on_event: impl FnMut(Option<&A>) + Send + 'static,
) -> TypedEventHandler<S, A>
where
    S: RuntimeType + 'static,
    A: RuntimeType + 'static,
{
    TypedEventHandler::new(move |_: Ref<'_, S>, args: Ref<'_, A>| {
        on_event(args.as_ref());
        Ok(())
    })
}

/// Like [`handler`], for an event whose sender carries the news.
pub(super) fn sender_handler<S, A>(
    mut on_event: impl FnMut(Option<&S>) + Send + 'static,
) -> TypedEventHandler<S, A>
where
    S: RuntimeType + 'static,
    A: RuntimeType + 'static,
{
    TypedEventHandler::new(move |sender: Ref<'_, S>, _: Ref<'_, A>| {
        on_event(sender.as_ref());
        Ok(())
    })
}

/// The Windows device ID of the remote end of a session. It never leaves the backend.
pub(super) fn session_device(session: windows::core::Result<GattSession>) -> Option<String> {
    session
        .and_then(|session| session.DeviceId())
        .and_then(|device| device.Id())
        .ok()
        .map(|id| id.to_string_lossy())
}

pub(super) fn bytes(buffer: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let reader = DataReader::FromBuffer(buffer)?;
    let mut bytes = vec![0; reader.UnconsumedBufferLength()? as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn buffer(bytes: &[u8]) -> windows::core::Result<IBuffer> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(bytes)?;
    writer.DetachBuffer()
}

pub(super) fn guid(uuid: &str) -> Option<GUID> {
    uuid_to_u128(uuid).map(GUID::from_u128)
}

pub(super) fn uuid(guid: GUID) -> String {
    uuid_from_u128(guid.to_u128())
}

/// The items of a vector view, without the iterator that panics when `First` fails.
pub(super) fn items<T: RuntimeType + 'static>(
    view: &IVectorView<T>,
) -> windows::core::Result<Vec<T>> {
    (0..view.Size()?).map(|index| view.GetAt(index)).collect()
}

/// The ATT error of a GATT result, which is null when there is none.
pub(super) fn att_error(reference: windows::core::Result<IReference<u8>>) -> Option<u8> {
    reference.ok().and_then(|reference| reference.Value().ok())
}

/// A failed `WinRT` call, as a contract error that keeps its `HRESULT`.
pub(super) fn winrt_error(error: &Error, action: &str) -> BleError {
    hresult_error(error.code().0, &error.message(), action)
}

/// Like [`winrt_error`], but a null result, which `windows` reports as an
/// error with a success code, rejects with `code` and `missing`.
pub(super) fn winrt_or_missing(
    error: &Error,
    action: &str,
    code: ErrorCode,
    missing: &str,
) -> BleError {
    if error.code().is_ok() {
        BleError::new(code, missing)
    } else {
        winrt_error(error, action)
    }
}
