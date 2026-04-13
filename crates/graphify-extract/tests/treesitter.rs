//! Integration tests for tree-sitter based extraction.

use graphify_core::model::NodeType;
use graphify_extract::treesitter::try_extract;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════════
// Python
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_python_extracts_class_and_methods() {
    let source = br#"
class MyClass:
    def __init__(self):
        pass

    def greet(self, name):
        return f"Hello {name}"

def standalone():
    pass
"#;
    let result = try_extract(Path::new("test.py"), source, "python").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("MyClass")));
    assert!(labels.iter().any(|l| l.contains("__init__")));
    assert!(labels.iter().any(|l| l.contains("greet")));
    assert!(labels.iter().any(|l| l.contains("standalone")));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Class));
}

#[test]
fn ts_python_extracts_imports() {
    let source = br#"
import os
from pathlib import Path
from collections import defaultdict, OrderedDict
"#;
    let result = try_extract(Path::new("test.py"), source, "python").unwrap();
    let import_count = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .count();
    assert!(
        import_count >= 2,
        "expected >=2 imports, got {import_count}"
    );
}

#[test]
fn ts_python_infers_calls() {
    let source = br#"
def foo():
    bar()

def bar():
    pass
"#;
    let result = try_extract(Path::new("test.py"), source, "python").unwrap();
    assert!(result.edges.iter().any(|e| e.relation == "calls"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Rust
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_rust_extracts_structs_and_functions() {
    let source = br#"
use std::collections::HashMap;

pub struct Config { name: String }
pub enum Status { Active, Inactive }
pub trait Runnable { fn run(&self); }

impl Runnable for Config {
    fn run(&self) { println!("{}", self.name); }
}

pub fn main() {
    let c = Config { name: "test".into() };
    c.run();
}
"#;
    let result = try_extract(Path::new("lib.rs"), source, "rust").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Config")));
    assert!(labels.iter().any(|l| l.contains("Status")));
    assert!(labels.iter().any(|l| l.contains("Runnable")));
    assert!(labels.iter().any(|l| l.contains("main")));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Enum));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Trait));
    assert!(result.edges.iter().any(|e| e.relation == "implements"));
}

// ═══════════════════════════════════════════════════════════════════════════
// JavaScript
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_js_extracts_functions_and_classes() {
    let source = br#"
import { useState } from 'react';
import axios from 'axios';

export class ApiClient {
    constructor(baseUrl) { this.baseUrl = baseUrl; }
}

export function fetchData(url) { return axios.get(url); }
"#;
    let result = try_extract(Path::new("api.js"), source, "javascript").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("ApiClient")));
    assert!(labels.iter().any(|l| l.contains("fetchData")));
    assert!(
        result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count()
            >= 2
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Go
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_go_extracts_types_and_functions() {
    let source = br#"
package main

import (
    "fmt"
    "os"
)

type Server struct { host string; port int }
type Handler interface { Handle() }

func (s *Server) Start() { fmt.Println("starting") }
func main() { s := Server{host: "localhost", port: 8080}; s.Start() }
"#;
    let result = try_extract(Path::new("main.go"), source, "go").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Server")));
    assert!(labels.iter().any(|l| l.contains("Handler")));
    assert!(labels.iter().any(|l| l.contains("Start")));
    assert!(labels.iter().any(|l| l.contains("main")));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.node_type == NodeType::Interface)
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Unsupported & comparison
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_unsupported_returns_none() {
    assert!(try_extract(Path::new("test.pl"), b"sub foo { 1 }", "perl").is_none());
}

#[test]
fn ts_python_at_least_as_many_nodes_as_regex() {
    let source_str = r#"
class MyClass:
    def __init__(self):
        pass

    def greet(self, name):
        return f"Hello {name}"

def standalone():
    pass
"#;
    let regex_result =
        graphify_extract::ast_extract::extract_file(Path::new("test.py"), source_str, "python");
    let ts_result = try_extract(Path::new("test.py"), source_str.as_bytes(), "python").unwrap();
    assert!(ts_result.nodes.len() >= regex_result.nodes.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// Java
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_java_extracts_class_and_methods() {
    let source = br#"
import java.util.List;

public class Foo {
    public void bar() {}
    public int baz(String s) { return 0; }
}
"#;
    let result = try_extract(Path::new("Foo.java"), source, "java").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Foo")));
    assert!(labels.iter().any(|l| l.contains("bar")));
    assert!(labels.iter().any(|l| l.contains("baz")));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn ts_java_extracts_interface() {
    let source = br#"
public interface Runnable { void run(); }
"#;
    let result = try_extract(Path::new("Runnable.java"), source, "java").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Runnable")));
}

// ═══════════════════════════════════════════════════════════════════════════
// C / C++ / Ruby / C# / Dart
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ts_c_extracts_functions() {
    let source = br#"
#include <stdio.h>
int main(int argc, char **argv) { printf("hello\n"); return 0; }
void helper(void) {}
"#;
    let result = try_extract(Path::new("main.c"), source, "c").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("main")));
    assert!(labels.iter().any(|l| l.contains("helper")));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn ts_cpp_extracts_class_and_functions() {
    let source = br#"
#include <iostream>

class Greeter {
public:
    void greet() { std::cout << "hello" << std::endl; }
};

int main() { Greeter g; g.greet(); return 0; }
"#;
    let result = try_extract(Path::new("main.cpp"), source, "cpp").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Greeter")));
    assert!(labels.iter().any(|l| l.contains("main")));
}

#[test]
fn ts_ruby_extracts_class_and_methods() {
    let source = br#"
class Dog
  def initialize(name)
    @name = name
  end
  def bark
    puts "Woof!"
  end
end
"#;
    let result = try_extract(Path::new("dog.rb"), source, "ruby").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Dog")));
    assert!(labels.iter().any(|l| l.contains("initialize")));
    assert!(labels.iter().any(|l| l.contains("bark")));
}

#[test]
fn ts_csharp_extracts_class_and_methods() {
    let source = br#"
using System;
using System.Collections.Generic;

public class Calculator {
    public int Add(int a, int b) { return a + b; }
    public int Subtract(int a, int b) { return a - b; }
}
"#;
    let result = try_extract(Path::new("Calculator.cs"), source, "csharp").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("Calculator")));
    assert!(labels.iter().any(|l| l.contains("Add")));
    assert!(labels.iter().any(|l| l.contains("Subtract")));
    assert!(
        result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count()
            >= 2
    );
}

#[test]
fn ts_dart_extracts_class_and_methods() {
    let source = br#"
import 'dart:async';
import 'package:flutter/material.dart';

enum Status { active, inactive }

void main() { print('hello'); }
"#;
    let result = try_extract(Path::new("user_service.dart"), source, "dart").unwrap();
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.contains("main")));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
    assert!(
        result
            .edges
            .iter()
            .filter(|e| e.relation == "imports")
            .count()
            >= 2
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Cross-cutting
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn all_edges_have_source_file() {
    let source = b"def foo():\n    bar()\ndef bar():\n    pass\n";
    let result = try_extract(Path::new("x.py"), source, "python").unwrap();
    for edge in &result.edges {
        assert!(!edge.source_file.is_empty());
    }
}

#[test]
fn node_ids_are_deterministic() {
    let source = b"def foo():\n    pass\n";
    let r1 = try_extract(Path::new("test.py"), source, "python").unwrap();
    let r2 = try_extract(Path::new("test.py"), source, "python").unwrap();
    assert_eq!(r1.nodes.len(), r2.nodes.len());
    for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
        assert_eq!(a.id, b.id);
    }
}
