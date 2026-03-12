# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly.

**Do not open a public issue.**

Instead, email **teo@bouancheau.com** with:

- Description of the vulnerability
- Steps to reproduce
- Impact assessment

You will receive a response within 48 hours. We will work with you to understand and address the issue before any public disclosure.

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 1.x     | Yes       |

## Scope

TokenStunt runs locally and processes code on your machine. Security concerns include:

- SQL injection via crafted file names or code content
- Path traversal in file walker
- Denial of service via malformed tree-sitter input
- Credential exposure in config files
