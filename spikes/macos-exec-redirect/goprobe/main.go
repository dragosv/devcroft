// Task 0.2: does the interposition library reach a Go program's execs?
// Go on darwin calls libSystem through trampolines, so each case below
// either reaches the runtime (redirected) or fails with EPERM.
package main

import (
	"fmt"
	"os"
	"os/exec"
	"syscall"
)

func run(label string, name string, args ...string) {
	out, err := exec.Command(name, args...).CombinedOutput()
	fmt.Printf("-- go %s: %q err=%v\n", label, string(out), err)
}

func main() {
	run("exec.Command(/bin/sh -c)", "/bin/sh", "-c", "echo go-sh:${BASH_VERSION:-?}")
	run("exec.Command(#!/bin/sh script)", "./s.sh")
	run("exec.Command(/bin/echo)", "/bin/echo", "go-echo-ok")
	run("exec.Command(/bin/ls)", "/bin/ls", "/")
	if len(os.Args) > 1 && os.Args[1] == "exec" {
		// syscall.Exec replaces the process: the last case.
		err := syscall.Exec("/bin/sh", []string{"sh", "-c", "echo go-syscall-exec:${BASH_VERSION:-?}"}, os.Environ())
		fmt.Printf("-- go syscall.Exec(/bin/sh): err=%v\n", err)
	}
}
