// Scratch harness: the PwrFinicky router's Resolve with an opener, which the
// pwrfinicky --test CLI cannot pass. One URL per process.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"os"
	"strings"

	"finicky/resolver"
	"finicky/router"
)

func main() {
	dataDir := flag.String("data-dir", "", "")
	configPath := flag.String("config", "", "")
	opener := flag.String("opener", "", "NAME|BUNDLE_ID|PATH")
	testURL := flag.String("test", "", "")
	flag.Parse()
	slog.SetDefault(slog.New(slog.NewJSONHandler(io.Discard, nil)))
	engine, err := router.New(router.Options{DataDir: *dataDir, ConfigPath: *configPath, Version: "oracle", DryRun: true})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	defer engine.Close()
	var info *resolver.OpenerInfo
	if *opener != "" {
		parts := strings.SplitN(*opener, "|", 3)
		for len(parts) < 3 {
			parts = append(parts, "")
		}
		info = &resolver.OpenerInfo{Name: parts[0], BundleID: parts[1], Path: parts[2]}
	}
	result, err := engine.Resolve(*testURL, info, false)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	json.NewEncoder(os.Stdout).Encode(result)
}
