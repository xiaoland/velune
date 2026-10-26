#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
swiftc app/mac/TokenCount.swift scripts/check-token-units.swift -o "$work/swift-check"
"$work/swift-check"
cp app/windows/TokenCount.cs scripts/check-token-units.cs "$work/"
sdk_major=$(dotnet --version | cut -d. -f1)
cat > "$work/check.csproj" <<PROJECT
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType><TargetFramework>net${sdk_major}.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings><Nullable>enable</Nullable><TreatWarningsAsErrors>true</TreatWarningsAsErrors>
  </PropertyGroup>
</Project>
PROJECT
dotnet run --project "$work/check.csproj"
