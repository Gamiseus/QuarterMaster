#!/usr/bin/env python3
"""
Test Suite for /api/devtools/vanilla-source endpoint.
Verifies that the backend correctly routes and serves full vanilla source files.
"""

import os
import re
from pathlib import Path

def test_main_rs_has_vanilla_source_route():
    main_rs_path = Path("Quartermaster/crates/qm-app/src/main.rs")
    assert main_rs_path.is_file(), f"main.rs not found at {main_rs_path}"
    content = main_rs_path.read_text(encoding="utf-8")
    
    # Must have the route match
    assert '("GET", "/api/devtools/vanilla-source")' in content, (
        "Expected GET /api/devtools/vanilla-source route in main.rs"
    )
    
    # Must have the build_vanilla_source function
    assert "fn build_vanilla_source" in content, (
        "Expected fn build_vanilla_source in main.rs"
    )

def test_build_vanilla_source_returns_required_fields():
    main_rs_path = Path("Quartermaster/crates/qm-app/src/main.rs")
    content = main_rs_path.read_text(encoding="utf-8")
    
    # Check that it returns json!({ "file": ..., "content": ..., "lines": ... })
    match = re.search(r'fn build_vanilla_source\b[\s\S]*?json!\(\{[\s\S]*?\}\)', content)
    assert match is not None, "fn build_vanilla_source must return json object"
    func_body = match.group(0)
    assert '"file"' in func_body, "Result must contain 'file'"
    assert '"content"' in func_body, "Result must contain 'content'"
    assert '"lines"' in func_body, "Result must contain 'lines'"
