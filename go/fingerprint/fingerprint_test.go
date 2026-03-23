package fingerprint

import (
	"errors"
	"fmt"
	"testing"
)

func TestVerifyRejectsEmptyTemplates(t *testing.T) {
	_, err := Verify(nil, []byte{1, 2, 3})
	if err == nil {
		t.Fatalf("expected error for empty template A")
	}

	_, err = Verify([]byte{1, 2, 3}, nil)
	if err == nil {
		t.Fatalf("expected error for empty template B")
	}
}

func TestCloseNilDeviceIsSafe(t *testing.T) {
	var d *Device
	d.Close()
}

func TestMatchThresholdDefaultAndEnv(t *testing.T) {
	t.Setenv("FP_MATCH_THRESHOLD", "")
	if got := MatchThreshold(); got != DefaultMatchThreshold {
		t.Fatalf("expected default threshold %.4f, got %.4f", DefaultMatchThreshold, got)
	}

	t.Setenv("FP_MATCH_THRESHOLD", "0.07")
	if got := MatchThreshold(); got != 0.07 {
		t.Fatalf("expected env threshold 0.07, got %.4f", got)
	}

	t.Setenv("FP_MATCH_THRESHOLD", "invalid")
	if got := MatchThreshold(); got != DefaultMatchThreshold {
		t.Fatalf("expected fallback threshold %.4f, got %.4f", DefaultMatchThreshold, got)
	}
}

func TestIsMatchUsesConfiguredThreshold(t *testing.T) {
	t.Setenv("FP_MATCH_THRESHOLD", "0.05")
	if !IsMatch(0.05) {
		t.Fatalf("expected score at threshold to match")
	}
	if IsMatch(0.049) {
		t.Fatalf("expected score below threshold to not match")
	}
}

func TestSelectBestTemplateRejectsTooFewTemplates(t *testing.T) {
	_, err := SelectBestTemplate(nil)
	if !errors.Is(err, ErrInsufficientTemplates) {
		t.Fatalf("expected ErrInsufficientTemplates, got %v", err)
	}
}

func TestSelectBestTemplatePicksHighestAverage(t *testing.T) {
	tmplA := []byte{0xA1}
	tmplB := []byte{0xB2}
	tmplC := []byte{0xC3}

	scoreMap := map[string]float64{
		"161-178": 0.30, // A-B
		"178-161": 0.30,
		"161-195": 0.90, // A-C
		"195-161": 0.90,
		"178-195": 0.20, // B-C
		"195-178": 0.20,
	}

	scorer := func(a, b []byte) (float64, error) {
		if len(a) == 0 || len(b) == 0 {
			return 0, ErrEmptyTemplate
		}
		key := fmt.Sprintf("%d-%d", a[0], b[0])
		score, ok := scoreMap[key]
		if !ok {
			return 0, fmt.Errorf("missing score for %s", key)
		}
		return score, nil
	}

	got, err := selectBestTemplateWithScorer([][]byte{tmplA, tmplB, tmplC}, scorer)
	if err != nil {
		t.Fatalf("select best template failed: %v", err)
	}
	if len(got) != 1 || got[0] != tmplA[0] {
		t.Fatalf("expected template A to win, got %v", got)
	}
}

func TestRecoverableCaptureErrorClassification(t *testing.T) {
	recoverable := []ErrorCode{ErrTimeout, ErrNoFinger, ErrImageInvalid, ErrExtractFail}
	for _, code := range recoverable {
		if !isRecoverableCaptureError(FpError{Code: code}) {
			t.Fatalf("expected code %d to be recoverable", code)
		}
	}

	if isRecoverableCaptureError(FpError{Code: ErrUsbIO}) {
		t.Fatalf("did not expect ErrUsbIO to be recoverable")
	}
}
