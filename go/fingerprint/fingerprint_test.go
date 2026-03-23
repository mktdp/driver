package fingerprint

import "testing"

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
