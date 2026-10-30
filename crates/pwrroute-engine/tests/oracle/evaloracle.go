// Scratch harness: runs a script in the PwrFinicky Go host's goja VM, set up
// as the host sets one up (`self`, `console`, the config API and the
// `finicky` global), and prints what an expression evaluates to as JSON.
// record.py runs tests/configs/finicky-unit.js through it.
//
//	evaloracle --api finickyConfigAPI.js --script finicky-unit.js --expr 'finickyUnit.run()'
package main

import (
	"flag"
	"fmt"
	"io"
	"log/slog"
	"os"

	"finicky/config"
)

func main() {
	apiPath := flag.String("api", "", "finickyConfigAPI.js")
	scriptPath := flag.String("script", "", "")
	expr := flag.String("expr", "", "")
	flag.Parse()
	slog.SetDefault(slog.New(slog.NewJSONHandler(io.Discard, nil)))
	api, err := os.ReadFile(*apiPath)
	if err != nil {
		fail(err)
	}
	script, err := os.ReadFile(*scriptPath)
	if err != nil {
		fail(err)
	}
	vm, err := config.NewFromScript(api, "finickyConfig", `var finickyConfig = { default: { defaultBrowser: "Home Browser" } };`)
	if err != nil {
		fail(err)
	}
	runtime := vm.Runtime()
	if _, err := runtime.RunString(string(script)); err != nil {
		fail(err)
	}
	out, err := runtime.RunString("JSON.stringify(" + *expr + ")")
	if err != nil {
		fail(err)
	}
	fmt.Println(out.String())
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
