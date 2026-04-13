//! Integration tests for regex-based AST extraction across all supported languages.

use graphify_core::confidence::Confidence;
use graphify_core::model::NodeType;
use graphify_extract::ast_extract::extract_file;
use std::path::Path;

// ═══════════════════════════════════════════════════════════════════════════
// Python
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn python_extracts_class_and_methods() {
    let source = r#"
class MyClass:
    def __init__(self):
        pass

    def greet(self, name):
        return f"Hello {name}"

def standalone():
    pass
"#;
    let result = extract_file(Path::new("test.py"), source, "python");

    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"MyClass"), "missing MyClass: {labels:?}");
    assert!(labels.contains(&"__init__"), "missing __init__: {labels:?}");
    assert!(labels.contains(&"greet"), "missing greet: {labels:?}");
    assert!(
        labels.contains(&"standalone"),
        "missing standalone: {labels:?}"
    );
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Class));
}

#[test]
fn python_extracts_imports() {
    let source = r#"
import os
from pathlib import Path
from collections import defaultdict, OrderedDict
"#;
    let result = extract_file(Path::new("test.py"), source, "python");
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
fn python_infers_calls() {
    let source = "def foo():\n    bar()\n\ndef bar():\n    pass\n";
    let result = extract_file(Path::new("test.py"), source, "python");
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();
    assert!(!call_edges.is_empty(), "expected call edges");
    assert_eq!(call_edges[0].confidence, Confidence::Inferred);
}

// ═══════════════════════════════════════════════════════════════════════════
// Rust
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rust_extracts_structs_and_functions() {
    let source = r#"
use std::collections::HashMap;

pub struct Config {
    name: String,
}

pub enum Status {
    Active,
    Inactive,
}

pub trait Runnable {
    fn run(&self);
}

impl Runnable for Config {
    fn run(&self) {
        println!("{}", self.name);
    }
}

pub fn main() {
    let c = Config { name: "test".into() };
    c.run();
}
"#;
    let result = extract_file(Path::new("lib.rs"), source, "rust");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Config"), "missing Config");
    assert!(labels.contains(&"Status"), "missing Status");
    assert!(labels.contains(&"Runnable"), "missing Runnable");
    assert!(labels.contains(&"main"), "missing main");
    assert!(labels.contains(&"run"), "missing run");
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Enum));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Trait));
    assert!(result.edges.iter().any(|e| e.relation == "implements"));
    assert!(result.nodes.iter().any(|n| n.label.contains("std")));
}

// ═══════════════════════════════════════════════════════════════════════════
// JavaScript / TypeScript
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn js_extracts_functions_and_classes() {
    let source = r#"
import { useState } from 'react';
import axios from 'axios';

export class ApiClient {
    constructor(baseUrl) {
        this.baseUrl = baseUrl;
    }
}

export function fetchData(url) {
    return axios.get(url);
}
"#;
    let result = extract_file(Path::new("api.js"), source, "javascript");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"ApiClient"));
    assert!(labels.contains(&"fetchData"));
    let import_count = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .count();
    assert!(import_count >= 2);
}

#[test]
fn ts_extracts_same_as_js() {
    let source = "export function hello(): string { return 'hi'; }\n";
    let result = extract_file(Path::new("hello.ts"), source, "typescript");
    assert!(result.nodes.iter().any(|n| n.label == "hello"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Go
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn go_extracts_types_and_functions() {
    let source = r#"
package main

import (
    "fmt"
    "os"
)

type Server struct {
    host string
    port int
}

type Handler interface {
    Handle()
}

func (s *Server) Start() {
    fmt.Println("starting")
}

func main() {
    s := Server{host: "localhost", port: 8080}
    s.Start()
}
"#;
    let result = extract_file(Path::new("main.go"), source, "go");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Server"));
    assert!(labels.contains(&"Handler"));
    assert!(labels.contains(&"Start"));
    assert!(labels.contains(&"main"));
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.node_type == NodeType::Interface)
    );
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
    assert!(result.nodes.iter().any(|n| n.label == "fmt"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Java
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn java_extracts_class_and_methods() {
    let source = r#"
import java.util.List;
import java.util.ArrayList;

public class UserService {
    private List<String> users;

    public UserService() {
        this.users = new ArrayList<>();
    }

    public void addUser(String name) {
        users.add(name);
    }

    public List<String> getUsers() {
        return users;
    }
}
"#;
    let result = extract_file(Path::new("UserService.java"), source, "java");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"UserService"));
    assert!(labels.contains(&"addUser"));
    assert!(labels.contains(&"getUsers"));
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
fn java_extracts_interface_and_enum() {
    let source = r#"
public interface Serializable {
    byte[] serialize();
}

public enum Status {
    ACTIVE,
    INACTIVE,
    PENDING;
}

public class Handler {
    public void handle(Status s) {}
}
"#;
    let result = extract_file(Path::new("Types.java"), source, "java");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Serializable"));
    assert!(labels.contains(&"Status"));
    assert!(labels.contains(&"Handler"));
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.node_type == NodeType::Interface)
    );
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Enum));
}

// ═══════════════════════════════════════════════════════════════════════════
// C / C++
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c_extracts_includes_and_functions() {
    let source = r#"
#include <stdio.h>
#include "myheader.h"

typedef struct Point { int x; int y; } Point;

int add(int a, int b) { return a + b; }
int main() { return 0; }
"#;
    let result = extract_file(Path::new("main.c"), source, "c");
    assert!(result.edges.iter().any(|e| e.relation == "includes"));
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"main"));
    assert!(labels.contains(&"add"));
}

#[test]
fn c_extracts_structs() {
    let source = r#"
#include <stdlib.h>

struct Vector { double x; double y; double z; };

typedef struct Matrix { double data[16]; } Matrix;

void init_vector(struct Vector *v) { v->x = 0; }
"#;
    let result = extract_file(Path::new("types.c"), source, "c");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Vector"));
    assert!(labels.contains(&"Matrix"));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
}

#[test]
fn cpp_extracts_class_and_namespace() {
    let source = r#"
#include <iostream>
#include <string>

namespace MyApp {
class Logger {
public:
    void log(const std::string& msg) {}
};
struct Config { std::string host; int port; };
}
"#;
    let result = extract_file(Path::new("logger.cpp"), source, "cpp");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"MyApp"));
    assert!(labels.contains(&"Logger"));
    assert!(labels.contains(&"Config"));
    assert!(result.edges.iter().any(|e| e.relation == "includes"));
}

// ═══════════════════════════════════════════════════════════════════════════
// C#
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn csharp_extracts_class_and_methods() {
    let source = r#"
using System;
using System.Collections.Generic;

public interface IRepository { void Save(object item); }
public enum Priority { Low, Medium, High }
public struct Coordinate { public double X; public double Y; }

public class Calculator {
    public int Add(int a, int b) { return a + b; }
    public int Subtract(int a, int b) { return a - b; }
}
"#;
    let result = extract_file(Path::new("Calculator.cs"), source, "csharp");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Calculator"));
    assert!(labels.contains(&"Add"));
    assert!(labels.contains(&"IRepository"));
    assert!(labels.contains(&"Priority"));
    assert!(labels.contains(&"Coordinate"));
    assert!(
        result
            .nodes
            .iter()
            .any(|n| n.node_type == NodeType::Interface)
    );
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Enum));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::Struct));
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
fn csharp_infers_calls() {
    let source = r#"
public class Service
{
    public void Run()
    {
        Process();
    }

    public void Process()
    {
        Console.WriteLine("done");
    }
}
"#;
    let result = extract_file(Path::new("Service.cs"), source, "csharp");
    assert!(result.edges.iter().any(|e| e.relation == "calls"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Ruby
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ruby_extracts_class_and_methods() {
    let source = r#"
require 'json'
require_relative 'helpers'

module Utilities
  class Greeter
    def initialize(name)
      @name = name
    end
    def greet
      "Hello, #{@name}!"
    end
  end
end
"#;
    let result = extract_file(Path::new("greeter.rb"), source, "ruby");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Utilities"));
    assert!(labels.contains(&"Greeter"));
    assert!(labels.contains(&"initialize"));
    assert!(labels.contains(&"greet"));
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
// Kotlin
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kotlin_extracts_class_and_functions() {
    let source = r#"
import kotlin.math.sqrt
import kotlin.collections.mutableListOf

data class Point(val x: Double, val y: Double)
interface Measurable { fun measure(): Double }
object Constants { val PI = 3.14159 }

fun distance(a: Point, b: Point): Double {
    return sqrt((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y))
}
"#;
    let result = extract_file(Path::new("geometry.kt"), source, "kotlin");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Point"));
    assert!(labels.contains(&"Measurable"));
    assert!(labels.contains(&"Constants"));
    assert!(labels.contains(&"distance"));
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
fn kotlin_infers_calls() {
    let source = r#"
fun fetchData(url: String): String { return processData(url) }
fun processData(input: String): String { return input.uppercase() }
"#;
    let result = extract_file(Path::new("service.kt"), source, "kotlin");
    assert!(result.edges.iter().any(|e| e.relation == "calls"));
}

// ═══════════════════════════════════════════════════════════════════════════
// Generic fallback languages (Scala, PHP, Swift, Lua, Zig, PowerShell,
// Elixir, Objective-C, Julia)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn scala_extracts_class_and_functions() {
    let source = r#"
import scala.collection.mutable

class Calculator {
  def add(a: Int, b: Int): Int = a + b
  def subtract(a: Int, b: Int): Int = a - b
}

object Main {
  def main(args: Array[String]): Unit = {
    val calc = new Calculator()
    println(calc.add(1, 2))
  }
}

trait Printable { def print(): Unit }
enum Color { case Red, Green, Blue }
"#;
    let result = extract_file(Path::new("Main.scala"), source, "scala");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Calculator"));
    assert!(labels.contains(&"add"));
    assert!(labels.contains(&"main"));
    assert!(labels.contains(&"Main"));
    assert!(labels.contains(&"Printable"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn php_extracts_class_and_functions() {
    let source = r#"<?php
namespace App\Models;
use Illuminate\Database\Eloquent\Model;

interface Authenticatable { public function getAuthId(): string; }

class User extends Model {
    public function getName(): string { return $this->name; }
    public function setName(string $name): void { $this->name = $name; }
}

trait HasTimestamps {
    public function getCreatedAt(): string { return $this->created_at; }
}

function helper(): void { echo "hello"; }
"#;
    let result = extract_file(Path::new("User.php"), source, "php");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"User"));
    assert!(labels.contains(&"getName"));
    assert!(labels.contains(&"helper"));
    assert!(labels.contains(&"Authenticatable"));
    assert!(labels.contains(&"HasTimestamps"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn swift_extracts_class_and_functions() {
    let source = r#"
import Foundation

protocol Fetchable { func fetch(id: Int) -> String }

class UserManager {
    func fetchUser(id: Int) -> String { return "User \(id)" }
    func deleteUser(id: Int) { print("Deleting \(id)") }
}

struct Config { let apiUrl: String; let timeout: Int }
enum AppState { case loading; case ready; case error }

func main() { let mgr = UserManager(); mgr.fetchUser(id: 1) }
"#;
    let result = extract_file(Path::new("UserManager.swift"), source, "swift");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"UserManager"));
    assert!(labels.contains(&"fetchUser"));
    assert!(labels.contains(&"main"));
    assert!(labels.contains(&"Config"));
    assert!(labels.contains(&"Fetchable"));
    assert!(labels.contains(&"AppState"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn lua_extracts_functions() {
    let source = r#"
require 'socket'

function greet(name)
    print("Hello " .. name)
end

function calculate(a, b)
    return a + b
end
"#;
    let result = extract_file(Path::new("module.lua"), source, "lua");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"greet"));
    assert!(labels.contains(&"calculate"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn zig_extracts_functions() {
    let source = r#"
const std = @import("std");

pub fn add(a: i32, b: i32) i32 { return a + b; }
fn helper() void { std.debug.print("hello", .{}); }
"#;
    let result = extract_file(Path::new("main.zig"), source, "zig");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"add"));
    assert!(labels.contains(&"helper"));
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
}

#[test]
fn powershell_extracts_class_and_functions() {
    let source = r#"
using module ActiveDirectory

class Logger { [string]$Path }
class UserService { [void] Process() { Write-Host "processing" } }

function Get_Users() { return @() }
"#;
    let result = extract_file(Path::new("utils.ps1"), source, "powershell");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Logger"));
    assert!(labels.contains(&"UserService"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn elixir_extracts_module_and_functions() {
    let source = r#"
defmodule MyApp.Calculator do
  use GenServer

  def add(a, b) do
    a + b
  end

  def subtract(a, b) do
    a - b
  end

  defp internal_helper() do
    :ok
  end
end
"#;
    let result = extract_file(Path::new("calculator.ex"), source, "elixir");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"MyApp"));
    assert!(labels.contains(&"add"));
    assert!(labels.contains(&"subtract"));
    assert!(result.edges.iter().any(|e| e.relation == "imports"));
}

#[test]
fn objc_extracts_class_and_interface() {
    let source = r#"
#import <Foundation/Foundation.h>

@interface UserManager : NSObject
- (NSString *)fetchUser:(NSInteger)userId;
@end

@implementation UserManager
- (NSString *)fetchUser:(NSInteger)userId { return @"User"; }
@end
"#;
    let result = extract_file(Path::new("UserManager.m"), source, "objc");
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"UserManager"));
}

#[test]
fn julia_extracts_functions_and_structs() {
    let source = r#"
using LinearAlgebra
import Statistics

function greet(name::String)
    println("Hello $name")
end

function calculate(a::Int, b::Int)::Int
    return a + b
end

struct Config
    host::String
    port::Int
end

module MathUtils
    export add
    function add(a, b)
        return a + b
    end
end
"#;
    let result = extract_file(Path::new("app.jl"), source, "julia");
    let labels: Vec<&str> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"greet"));
    assert!(labels.contains(&"calculate"));
    assert!(labels.contains(&"Config"));
    assert!(labels.contains(&"MathUtils"));
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
// Cross-cutting concerns
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn generic_extracts_basic_patterns() {
    let source = r#"
defmodule MyApp.Worker do
  def start(args) do
    process(args)
  end

  def process(data) do
    IO.puts(data)
  end
end
"#;
    let result = extract_file(Path::new("worker.ex"), source, "elixir");
    assert!(!result.nodes.is_empty());
    assert!(result.nodes.iter().any(|n| n.node_type == NodeType::File));
}

#[test]
fn node_ids_are_deterministic() {
    let source = "def foo():\n    pass\n";
    let r1 = extract_file(Path::new("test.py"), source, "python");
    let r2 = extract_file(Path::new("test.py"), source, "python");
    assert_eq!(r1.nodes.len(), r2.nodes.len());
    for (a, b) in r1.nodes.iter().zip(r2.nodes.iter()) {
        assert_eq!(a.id, b.id);
    }
}

#[test]
fn all_edges_have_source_file() {
    let source = "def foo():\n    bar()\ndef bar():\n    pass\n";
    let result = extract_file(Path::new("x.py"), source, "python");
    for edge in &result.edges {
        assert!(!edge.source_file.is_empty());
    }
}
