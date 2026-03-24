package fingerprint

/*
#cgo CFLAGS: -I${SRCDIR}/../../include
#cgo linux LDFLAGS: -L${SRCDIR}/../../target/debug -lmktdp_driver -Wl,-rpath,${SRCDIR}/../../target/debug
#cgo darwin LDFLAGS: -L${SRCDIR}/../../target/debug -lmktdp_driver -Wl,-rpath,${SRCDIR}/../../target/debug
#cgo windows LDFLAGS: -L${SRCDIR}/../../target/debug -lmktdp_driver
#include "fingerprint.h"
*/
import "C"

import (
	"errors"
	"os"
	"runtime"
	"strconv"
	"time"
	"unsafe"
)

// DefaultTimeoutMs is the default timeout used by Enroll.
const DefaultTimeoutMs uint32 = 10_000

// DefaultMatchThreshold is the default score threshold for a match decision.
const DefaultMatchThreshold float64 = 0.06

// DefaultEnrollScanCount is the recommended number of same-finger scans for enrollment.
const DefaultEnrollScanCount = 6

// DefaultEnrollAttemptsPerScan is the max attempts per scan slot during enrollment.
const DefaultEnrollAttemptsPerScan = 4

const continuousRecoverableErrorDelay = 250 * time.Millisecond
const enrollRetryDelay = 900 * time.Millisecond
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

// ErrInvalidScanCount is returned when enrollment scan count is invalid.
var ErrInvalidScanCount = errors.New("scan count must be >= 2")

// ErrInvalidAttemptsPerScan is returned when attempts per scan is invalid.
var ErrInvalidAttemptsPerScan = errors.New("attempts per scan must be >= 1")

// ErrInsufficientTemplates is returned when consensus selection has <2 templates.
var ErrInsufficientTemplates = errors.New("at least 2 templates are required")

// ErrNilScanHandler is returned when ScanContinuously receives a nil callback.
var ErrNilScanHandler = errors.New("scan handler must not be nil")

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

type templateScorer func(a, b []byte) (float64, error)

// EnrollConsensus captures the same finger 6x and returns one opaque
// enrollment package from the native Rust implementation.
func EnrollConsensus(dev *Device) ([]byte, error) {
	return EnrollFromScans(dev, DefaultEnrollScanCount, DefaultTimeoutMs, DefaultEnrollAttemptsPerScan)
}

// EnrollFromScans captures the same finger repeatedly and returns one
// enrollment package assembled in native Rust from all captures.
//
// Typical usage:
//   - scans = 6
//   - timeoutMs = 10000
//   - maxAttemptsPerScan = 4
func EnrollFromScans(
	dev *Device,
	scans int,
	timeoutMs uint32,
	maxAttemptsPerScan int,
) ([]byte, error) {
	if dev == nil || dev.ptr == nil {
		return nil, ErrDeviceClosed
	}
	if scans < 2 {
		return nil, ErrInvalidScanCount
	}
	if maxAttemptsPerScan < 1 {
		return nil, ErrInvalidAttemptsPerScan
	}

	var tmpl *C.uint8_t
	var tmplLen C.uintptr_t

	rc := C.fp_enroll_multi(
		dev.ptr,
		C.uint32_t(timeoutMs),
		C.uint32_t(scans),
		C.uint32_t(maxAttemptsPerScan),
		&tmpl,
		&tmplLen,
	)
	if rc != C.int32_t(C.FP_OK) {
		return nil, FpError{Code: ErrorCode(rc)}
	}
	defer C.fp_free(tmpl, tmplLen)

	if uintptr(tmplLen) > uintptr(1<<31-1) {
		return nil, errors.New("template length exceeds C.GoBytes limit")
	}
	template := C.GoBytes(unsafe.Pointer(tmpl), C.int(uintptr(tmplLen)))
	runtime.KeepAlive(dev)

	return template, nil
}

// SelectBestTemplate picks the template whose average Verify score against all
// other candidates is highest. Useful if capture is done outside this package.
func SelectBestTemplate(templates [][]byte) ([]byte, error) {
	return selectBestTemplateWithScorer(templates, Verify)
}

// ScanContinuously keeps scanning while the device stays open and invokes
// onTemplate for each successful capture.
//
// Returning false from onTemplate stops scanning and returns nil.
// Recoverable capture errors (timeout/no finger/invalid image/extract fail) are
// ignored so the scanner can stay active for attendance-style workflows.
func ScanContinuously(dev *Device, timeoutMs uint32, onTemplate func([]byte) bool) error {
	if dev == nil || dev.ptr == nil {
		return ErrDeviceClosed
	}
	if onTemplate == nil {
		return ErrNilScanHandler
	}

	for {
		tmpl, err := EnrollWithTimeout(dev, timeoutMs)
		if err != nil {
			if isRecoverableCaptureError(err) {
				time.Sleep(continuousRecoverableErrorDelay)
				continue
			}
			return err
		}

		if !onTemplate(tmpl) {
			return nil
		}
	}
}

func selectBestTemplateWithScorer(templates [][]byte, scorer templateScorer) ([]byte, error) {
	if len(templates) < 2 {
		return nil, ErrInsufficientTemplates
	}

	bestIdx := -1
	bestAvg := -1.0

	for i := range templates {
		if len(templates[i]) == 0 {
			return nil, ErrEmptyTemplate
		}

		var sum float64
		var count int
		for j := range templates {
			if i == j {
				continue
			}
			score, err := scorer(templates[i], templates[j])
			if err != nil {
				return nil, err
			}
			sum += score
			count++
		}

		avg := sum / float64(count)
		if avg > bestAvg {
			bestAvg = avg
			bestIdx = i
		}
	}

	out := make([]byte, len(templates[bestIdx]))
	copy(out, templates[bestIdx])
	return out, nil
}

func isRecoverableCaptureError(err error) bool {
	fpErr, ok := err.(FpError)
	if !ok {
		return false
	}

	switch fpErr.Code {
	case ErrTimeout, ErrNoFinger, ErrImageInvalid, ErrExtractFail:
		return true
	default:
		return false
	}
}

// StrError maps a native ErrorCode to a stable human-readable message.
func StrError(code ErrorCode) string {
	msg := C.fp_strerror(C.int32_t(code))
	if msg == nil {
		return "unknown error"
	}
	return C.GoString(msg)
}
