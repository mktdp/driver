package fingerprint

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#cgo LDFLAGS: -L${SRCDIR}/../../target/debug -lfingerprint_driver -Wl,-rpath,${SRCDIR}/../../target/debug
#include "fingerprint.h"
*/
import "C"

import (
	"errors"
	"os"
	"runtime"
	"strconv"
	"unsafe"
)

// DefaultTimeoutMs is the default timeout used by Enroll.
const DefaultTimeoutMs uint32 = 10_000

// DefaultMatchThreshold is the default score threshold for a match decision.
const DefaultMatchThreshold float64 = 0.06
const envMatchThreshold = "FP_MATCH_THRESHOLD"

// ErrorCode is a direct mapping of the Rust C ABI error codes.
type ErrorCode int32

const (
	// OK indicates success.
	OK ErrorCode = ErrorCode(C.FP_OK)
	// ErrDeviceNotFound indicates no supported USB scanner was found.
	ErrDeviceNotFound ErrorCode = ErrorCode(C.FP_ERR_DEVICE_NOT_FOUND)
	// ErrUsbIO indicates a USB communication error.
	ErrUsbIO ErrorCode = ErrorCode(C.FP_ERR_USB_IO)
	// ErrTimeout indicates a timeout waiting for finger/input.
	ErrTimeout ErrorCode = ErrorCode(C.FP_ERR_TIMEOUT)
	// ErrNoFinger indicates no finger was detected in time.
	ErrNoFinger ErrorCode = ErrorCode(C.FP_ERR_NO_FINGER)
	// ErrImageInvalid indicates invalid/corrupt captured image data.
	ErrImageInvalid ErrorCode = ErrorCode(C.FP_ERR_IMAGE_INVALID)
	// ErrExtractFail indicates template extraction failure.
	ErrExtractFail ErrorCode = ErrorCode(C.FP_ERR_EXTRACT_FAIL)
	// ErrNullPtr indicates an invalid null pointer argument was passed.
	ErrNullPtr ErrorCode = ErrorCode(C.FP_ERR_NULL_PTR)
	// ErrPanic indicates Rust panic was caught at FFI boundary.
	ErrPanic ErrorCode = ErrorCode(C.FP_ERR_PANIC)
)

// ErrDeviceClosed is returned when Enroll is called with a nil/closed handle.
var ErrDeviceClosed = errors.New("fingerprint device is nil or closed")

// ErrEmptyTemplate is returned when Verify receives an empty template.
var ErrEmptyTemplate = errors.New("template is empty")

// FpError wraps an ErrorCode from the native library.
type FpError struct {
	Code ErrorCode
}

func (e FpError) Error() string {
	return StrError(e.Code)
}

// Device wraps an opaque native scanner handle.
type Device struct {
	ptr *C.FpDevice
}

// Open opens the first available U.are.U 4500 scanner.
func Open() (*Device, error) {
	ptr := C.fp_open()
	if ptr == nil {
		return nil, FpError{Code: ErrDeviceNotFound}
	}
	return &Device{ptr: ptr}, nil
}

// Close closes the scanner handle. It is safe to call multiple times.
func (d *Device) Close() {
	if d == nil || d.ptr == nil {
		return
	}
	C.fp_close(d.ptr)
	d.ptr = nil
}

// Enroll captures a fingerprint and returns a template with the default timeout.
func Enroll(dev *Device) ([]byte, error) {
	return EnrollWithTimeout(dev, DefaultTimeoutMs)
}

// EnrollWithTimeout captures a fingerprint and returns an opaque template.
func EnrollWithTimeout(dev *Device, timeoutMs uint32) ([]byte, error) {
	if dev == nil || dev.ptr == nil {
		return nil, ErrDeviceClosed
	}

	var tmpl *C.uint8_t
	var tmplLen C.uintptr_t

	rc := C.fp_scan_and_extract(
		dev.ptr,
		C.uint32_t(timeoutMs),
		&tmpl,
		&tmplLen,
	)
	if rc != C.int32_t(C.FP_OK) {
		return nil, FpError{Code: ErrorCode(rc)}
	}
	defer C.fp_free(tmpl, tmplLen)

	// C.GoBytes takes C.int, so guard the conversion.
	if uintptr(tmplLen) > uintptr(1<<31-1) {
		return nil, errors.New("template length exceeds C.GoBytes limit")
	}
	template := C.GoBytes(unsafe.Pointer(tmpl), C.int(uintptr(tmplLen)))
	runtime.KeepAlive(dev)

	return template, nil
}

// Verify compares two templates and returns a normalized similarity score [0, 1].
func Verify(a, b []byte) (float64, error) {
	if len(a) == 0 || len(b) == 0 {
		return 0, ErrEmptyTemplate
	}

	aPtr := (*C.uint8_t)(unsafe.Pointer(&a[0]))
	bPtr := (*C.uint8_t)(unsafe.Pointer(&b[0]))
	var score C.double

	rc := C.fp_verify(
		aPtr,
		C.uintptr_t(len(a)),
		bPtr,
		C.uintptr_t(len(b)),
		&score,
	)
	runtime.KeepAlive(a)
	runtime.KeepAlive(b)

	if rc != C.int32_t(C.FP_OK) {
		return 0, FpError{Code: ErrorCode(rc)}
	}

	return float64(score), nil
}

// MatchThreshold returns the configured match threshold.
//
// It reads FP_MATCH_THRESHOLD if set, otherwise returns DefaultMatchThreshold.
// Values outside [0,1] or parse failures fall back to DefaultMatchThreshold.
func MatchThreshold() float64 {
	raw := os.Getenv(envMatchThreshold)
	if raw == "" {
		return DefaultMatchThreshold
	}
	v, err := strconv.ParseFloat(raw, 64)
	if err != nil || v < 0.0 || v > 1.0 {
		return DefaultMatchThreshold
	}
	return v
}

// IsMatch applies the configured threshold to a score from Verify.
func IsMatch(score float64) bool {
	return score >= MatchThreshold()
}

// StrError maps a native ErrorCode to a stable human-readable message.
func StrError(code ErrorCode) string {
	msg := C.fp_strerror(C.int32_t(code))
	if msg == nil {
		return "unknown error"
	}
	return C.GoString(msg)
}
