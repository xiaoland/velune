package main

// Finds installed commands the way `where` would, without running any launcher.
//
// npm and pnpm install three launchers per command: `x.cmd`, `x.ps1` and an
// extensionless `sh` script. Pi's installation resolver reads only the `sh`
// script's literal target, so Pi uses that one; Codex and Claude Code start
// their binary directly, so they use a native `.exe`, else the `.cmd` launcher.

import (
	"bytes"
	"os"
	"path/filepath"
	"slices"
)

func searchPath() []string {
	directories := filepath.SplitList(os.Getenv("PATH"))
	// Package managers' global directories, for an app started before PATH was updated.
	for _, extra := range [][2]string{
		{"LOCALAPPDATA", "pnpm"},
		{"APPDATA", "npm"},
		{"LOCALAPPDATA", `Programs\OpenAI\Codex\bin`},
	} {
		if base := os.Getenv(extra[0]); base != "" {
			directories = append(directories, filepath.Join(base, extra[1]))
		}
	}
	var unique []string
	for _, directory := range directories {
		if filepath.IsAbs(directory) && !slices.Contains(unique, directory) {
			unique = append(unique, directory)
		}
	}
	return unique
}

func isFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

func isShLauncher(path string) bool {
	content, err := os.ReadFile(path)
	return err == nil && bytes.HasPrefix(content, []byte("#!/bin/sh"))
}

// commandCandidates are every installation of command a runtime family can be
// probed with, in PATH order.
func commandCandidates(family, command string) []string {
	var found []string
	for _, directory := range searchPath() {
		var chosen string
		if family == "pi" {
			if launcher := filepath.Join(directory, command); isFile(launcher) && isShLauncher(launcher) {
				chosen = launcher
			}
		} else {
			for _, path := range []string{filepath.Join(directory, command+".exe"), filepath.Join(directory, command+".cmd")} {
				if isFile(path) {
					chosen = path
					break
				}
			}
		}
		if chosen != "" && !slices.Contains(found, chosen) {
			found = append(found, chosen)
		}
	}
	return found
}

// nodeBinary is the first node.exe on PATH, or "".
func nodeBinary() string {
	for _, directory := range searchPath() {
		if path := filepath.Join(directory, "node.exe"); isFile(path) {
			return path
		}
	}
	return ""
}
