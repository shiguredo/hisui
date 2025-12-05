use std::borrow::Cow;

use crate::{CudaLibrary, sys};

/// エラー
#[derive(Debug, Clone)]
pub struct Error {
    function: &'static str,
    status_code: Option<u32>,
    status_name: Option<Cow<'static, str>>,
    status_message: Option<Cow<'static, str>>,
}

impl Error {
    // CUDA や NVIDIA Video Codec SDK ではなく、この crate 起因のエラーを構築するための関数
    pub(crate) fn new_custom(function: &'static str, message: &'static str) -> Self {
        Self {
            function,
            status_code: None,
            status_name: None,
            status_message: Some(Cow::Borrowed(message)),
        }
    }

    // CUDA 関連のエラーを生成するための関数
    fn new_cuda(code: u32, function: &'static str) -> Self {
        // 可能なら詳細情報を取得する
        let mut status_name = None;
        let mut status_message = None;
        if let Ok(lib) = CudaLibrary::load() {
            status_name = lib.cu_get_error_name(code).map(Cow::Owned);
            status_message = lib.cu_get_error_string(code).map(Cow::Owned);
        }

        Self {
            function,
            status_code: Some(code),
            status_name,
            status_message,
        }
    }

    /// CUDA エラーをチェックする
    pub(crate) fn check_cuda(status: u32, function: &'static str) -> Result<(), Error> {
        if status == sys::cudaError_enum_CUDA_SUCCESS {
            Ok(())
        } else {
            Err(Self::new_cuda(status, function))
        }
    }

    // NVENC 関連のエラーを生成するための関数（CUDA とはまたエラーコードの空間が別）
    fn new_nvenc(code: u32, function: &'static str) -> Self {
        Self {
            function,
            status_code: Some(code),
            status_name: get_nvencstatus_name(code).map(Cow::Borrowed),
            status_message: get_nvencstatus_message(code).map(Cow::Borrowed),
        }
    }

    /// NVENC エラーをチェックする
    pub(crate) fn check_nvenc(status: u32, function: &'static str) -> Result<(), Error> {
        if status == sys::_NVENCSTATUS_NV_ENC_SUCCESS {
            Ok(())
        } else {
            Err(Self::new_nvenc(status, function))
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}() failed", self.function)?;

        if let Some(code) = self.status_code {
            write!(f, "[status={code}]")?;
        }
        if self.status_name.is_some() || self.status_message.is_some() {
            write!(f, ": ")?;
        }

        if let Some(message) = &self.status_message {
            write!(f, "{message}")?;
        }

        if let Some(name) = &self.status_name {
            if self.status_message.is_some() {
                write!(f, " ({name})")?;
            } else {
                write!(f, "{name}")?;
            }
        }

        Ok(())
    }
}

impl std::error::Error for Error {}

fn get_nvencstatus_name(status: u32) -> Option<&'static str> {
    match status {
        sys::_NVENCSTATUS_NV_ENC_SUCCESS => Some("NV_ENC_SUCCESS"),
        sys::_NVENCSTATUS_NV_ENC_ERR_NO_ENCODE_DEVICE => Some("NV_ENC_ERR_NO_ENCODE_DEVICE"),
        sys::_NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_DEVICE => Some("NV_ENC_ERR_UNSUPPORTED_DEVICE"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_ENCODERDEVICE => {
            Some("NV_ENC_ERR_INVALID_ENCODERDEVICE")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_DEVICE => Some("NV_ENC_ERR_INVALID_DEVICE"),
        sys::_NVENCSTATUS_NV_ENC_ERR_DEVICE_NOT_EXIST => Some("NV_ENC_ERR_DEVICE_NOT_EXIST"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR => Some("NV_ENC_ERR_INVALID_PTR"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_EVENT => Some("NV_ENC_ERR_INVALID_EVENT"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM => Some("NV_ENC_ERR_INVALID_PARAM"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_CALL => Some("NV_ENC_ERR_INVALID_CALL"),
        sys::_NVENCSTATUS_NV_ENC_ERR_OUT_OF_MEMORY => Some("NV_ENC_ERR_OUT_OF_MEMORY"),
        sys::_NVENCSTATUS_NV_ENC_ERR_ENCODER_NOT_INITIALIZED => {
            Some("NV_ENC_ERR_ENCODER_NOT_INITIALIZED")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_PARAM => Some("NV_ENC_ERR_UNSUPPORTED_PARAM"),
        sys::_NVENCSTATUS_NV_ENC_ERR_LOCK_BUSY => Some("NV_ENC_ERR_LOCK_BUSY"),
        sys::_NVENCSTATUS_NV_ENC_ERR_NOT_ENOUGH_BUFFER => Some("NV_ENC_ERR_NOT_ENOUGH_BUFFER"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_VERSION => Some("NV_ENC_ERR_INVALID_VERSION"),
        sys::_NVENCSTATUS_NV_ENC_ERR_MAP_FAILED => Some("NV_ENC_ERR_MAP_FAILED"),
        sys::_NVENCSTATUS_NV_ENC_ERR_NEED_MORE_INPUT => Some("NV_ENC_ERR_NEED_MORE_INPUT"),
        sys::_NVENCSTATUS_NV_ENC_ERR_ENCODER_BUSY => Some("NV_ENC_ERR_ENCODER_BUSY"),
        sys::_NVENCSTATUS_NV_ENC_ERR_EVENT_NOT_REGISTERD => Some("NV_ENC_ERR_EVENT_NOT_REGISTERD"),
        sys::_NVENCSTATUS_NV_ENC_ERR_GENERIC => Some("NV_ENC_ERR_GENERIC"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY => {
            Some("NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED => Some("NV_ENC_ERR_UNIMPLEMENTED"),
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_REGISTER_FAILED => {
            Some("NV_ENC_ERR_RESOURCE_REGISTER_FAILED")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_REGISTERED => {
            Some("NV_ENC_ERR_RESOURCE_NOT_REGISTERED")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_MAPPED => Some("NV_ENC_ERR_RESOURCE_NOT_MAPPED"),
        sys::_NVENCSTATUS_NV_ENC_ERR_NEED_MORE_OUTPUT => Some("NV_ENC_ERR_NEED_MORE_OUTPUT"),
        _ => None,
    }
}

fn get_nvencstatus_message(status: u32) -> Option<&'static str> {
    match status {
        sys::_NVENCSTATUS_NV_ENC_SUCCESS => Some("Encoding completed successfully"),
        sys::_NVENCSTATUS_NV_ENC_ERR_NO_ENCODE_DEVICE => {
            Some("No encode capable devices were detected")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_DEVICE => {
            Some("The devices passed by the client is not supported")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_ENCODERDEVICE => {
            Some("The encoder device supplied by the client is not valid")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_DEVICE => {
            Some("Device passed to the API call is invalid")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_DEVICE_NOT_EXIST => Some(
            "Device passed to the API call is no longer available and needs to be reinitialized",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PTR => {
            Some("One or more of the pointers passed to the API call is invalid")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_EVENT => {
            Some("Completion event passed in NvEncEncodePicture() call is invalid")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM => {
            Some("One or more of the parameter passed to the API call is invalid")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_CALL => {
            Some("An API call was made in wrong sequence/order")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_OUT_OF_MEMORY => Some(
            "The API call failed because it was unable to allocate enough memory to perform the requested operation",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_ENCODER_NOT_INITIALIZED => Some(
            "The encoder has not been initialized with NvEncInitializeEncoder() or initialization has failed",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_UNSUPPORTED_PARAM => {
            Some("An unsupported parameter was passed by the client")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_LOCK_BUSY => Some(
            "NvEncLockBitstream() failed to lock the output buffer. Retry after few milliseconds",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_NOT_ENOUGH_BUFFER => Some(
            "The size of the user buffer passed by the client is insufficient for the requested operation",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_VERSION => {
            Some("An invalid struct version was used by the client")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_MAP_FAILED => {
            Some("NvEncMapInputResource() API failed to map the client provided input resource")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_NEED_MORE_INPUT => Some(
            "Encode driver requires more input buffers to produce an output bitstream. This is not a fatal error",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_ENCODER_BUSY => Some(
            "The HW encoder is busy encoding and is unable to encode the input. Retry after few milliseconds",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_EVENT_NOT_REGISTERD => Some(
            "The completion event passed in NvEncEncodePicture() API has not been registered with encoder driver using NvEncRegisterAsyncEvent()",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_GENERIC => Some("An unknown internal error has occurred"),
        sys::_NVENCSTATUS_NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY => Some(
            "The client is attempting to use a feature that is not available for the license type for the current system",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_UNIMPLEMENTED => Some(
            "The client is attempting to use a feature that is not implemented for the current version",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_REGISTER_FAILED => {
            Some("NvEncRegisterResource API failed to register the resource")
        }
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_REGISTERED => Some(
            "The client is attempting to unregister a resource that has not been successfully registered",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_RESOURCE_NOT_MAPPED => Some(
            "The client is attempting to unmap a resource that has not been successfully mapped",
        ),
        sys::_NVENCSTATUS_NV_ENC_ERR_NEED_MORE_OUTPUT => Some(
            "Encode driver requires more output buffers to write an output bitstream. This is not a fatal error",
        ),

        // 不明なステータスの場合は None を返す
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_custom_display() {
        let error = Error::new_custom("test_func", "custom error message");
        assert_eq!(
            error.to_string(),
            "test_func() failed: custom error message"
        );
    }

    #[test]
    fn test_check_cuda_success() {
        let result = Error::check_cuda(sys::cudaError_enum_CUDA_SUCCESS, "cuda_func");
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_cuda_error() {
        let result = Error::check_cuda(1, "cuda_func");
        let error = result.expect_err("not err");

        assert!(
            error
                .to_string()
                .starts_with("cuda_func() failed[status=1]:")
        );

        // 具体的な内容は CUDA のバージョンなどに依存するので、存在することだけを確認する
        assert!(error.status_name.is_some());
        assert!(error.status_message.is_some());
    }

    #[test]
    fn test_check_nvenc_success() {
        let result = Error::check_nvenc(sys::_NVENCSTATUS_NV_ENC_SUCCESS, "nvenc_func");
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_nvenc_error() {
        let result = Error::check_nvenc(sys::_NVENCSTATUS_NV_ENC_ERR_INVALID_PARAM, "nvenc_func");
        let error = result.expect_err("not err");
        assert_eq!(
            error.to_string(),
            "nvenc_func() failed[status=2]: One or more of the parameter passed to the API call is invalid (NV_ENC_ERR_INVALID_PARAM)"
        );
    }
}
