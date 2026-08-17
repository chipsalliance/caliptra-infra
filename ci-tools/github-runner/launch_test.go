// Licensed under the Apache-2.0 license

package runner

import (
	"reflect"
	"testing"
)

func TestMachineInfoFromLabels(t *testing.T) {
	tests := []struct {
		name        string
		labels      []string
		expected    MachineInfo
		expectError bool
	}{
		{
			name:   "basic machine type",
			labels: []string{"self-hosted", "e2-standard-4"},
			expected: MachineInfo{
				machineType: "e2-standard-4",
			},
		},
		{
			name:   "machine with reporter suffix",
			labels: []string{"self-hosted", "e2-standard-4-reporter"},
			expected: MachineInfo{
				machineType: "e2-standard-4",
				isReporter:  true,
			},
		},
		{
			name:   "machine with reporter and big-disk",
			labels: []string{"self-hosted", "e2-standard-8-reporter-big-disk"},
			expected: MachineInfo{
				machineType: "e2-standard-8",
				hasBigDisk:  true,
				isReporter:  true,
			},
		},
		{
			name:   "machine with big-disk and reporter suffix",
			labels: []string{"self-hosted", "e2-standard-8-big-disk-reporter"},
			expected: MachineInfo{
				machineType: "e2-standard-8",
				hasBigDisk:  true,
				isReporter:  true,
			},
		},
		{
			name:   "machine with separate reporter label",
			labels: []string{"self-hosted", "e2-standard-4", "reporter"},
			expected: MachineInfo{
				machineType: "e2-standard-4",
				isReporter:  true,
			},
		},
		{
			name:   "machine with AI suffix",
			labels: []string{"self-hosted", "e2-standard-8-AI"},
			expected: MachineInfo{
				machineType: "e2-standard-8",
				isAI:        true,
			},
		},
		{
			name:   "machine with fpga-tools",
			labels: []string{"self-hosted", "e2-standard-16-fpga-tools"},
			expected: MachineInfo{
				machineType:  "e2-standard-16",
				hasFpgaTools: true,
			},
		},
		{
			name:   "machine with vck190-tools",
			labels: []string{"self-hosted", "e2-standard-32-vck190-tools"},
			expected: MachineInfo{
				machineType:    "e2-standard-32",
				hasVck190Tools: true,
			},
		},
		{
			name:        "missing machine type",
			labels:      []string{"self-hosted", "reporter"},
			expectError: true,
		},
		{
			name:        "multiple machine types",
			labels:      []string{"e2-standard-4", "e2-standard-8"},
			expectError: true,
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			actual, err := MachineInfoFromLabels(tc.labels)
			if tc.expectError {
				if err == nil {
					t.Fatalf("expected error, got nil")
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if !reflect.DeepEqual(actual, tc.expected) {
				t.Fatalf("mismatch: got %+v, want %+v", actual, tc.expected)
			}
		})
	}
}
