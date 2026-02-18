mod helpers;

use helpers::EditorTest;
use std::sync::atomic::{AtomicU64, Ordering};

fn temp_test_path(name: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("ovim_test_{}_{}", id, name))
        .to_string_lossy()
        .to_string()
}

/// Test TypeScript variable declaration and hover
#[test]
fn test_typescript_variable_declaration() {
    let code = r#"const greeting: string = "Hello";
let count: number = 42;
var flag: boolean = true;

console.log(greeting, count, flag);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'greeting' variable
    test.keys("0");
    test.keys("6l"); // Position on 'greeting'

    // Request hover (K) - should work even without LSP server running
    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);

    // Verify cursor is still on the variable
    test.assert_cursor(0, 6);
}

/// Test TypeScript function declaration
#[test]
fn test_typescript_function_declaration() {
    let code = r#"function add(a: number, b: number): number {
    return a + b;
}

const result = add(5, 3);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to function name
    test.keys("0");
    test.keys("9l"); // Position on 'add'

    // Attempt goto definition
    test.keys("gd");

    // Without LSP, cursor should not move
    test.assert_cursor(0, 9);
}

/// Test TypeScript arrow function
#[test]
fn test_typescript_arrow_function() {
    let code = r#"const multiply = (x: number, y: number): number => {
    return x * y;
};

const product = multiply(4, 5);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to multiply function call
    test.keys("5G0");
    test.keys("16l"); // Position on 'multiply' in call

    // Attempt goto definition
    test.keys("gd");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript class definition
#[test]
fn test_typescript_class_definition() {
    let code = r#"class Person {
    private name: string;
    private age: number;

    constructor(name: string, age: number) {
        this.name = name;
        this.age = age;
    }

    greet(): string {
        return `Hello, I'm ${this.name}`;
    }
}

const person = new Person("Alice", 30);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to class name
    test.keys("0");
    test.keys("6l"); // Position on 'Person'

    // Request hover
    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript interface definition
#[test]
fn test_typescript_interface_definition() {
    let code = r#"interface User {
    id: number;
    name: string;
    email: string;
}

const user: User = {
    id: 1,
    name: "Bob",
    email: "bob@example.com"
};
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to interface name (using w to move by word)
    test.keys("0w");

    // Store cursor position before gd
    let cursor_before = test.cursor();

    test.keys("gd");

    // Without LSP, cursor should not move
    assert_eq!(test.cursor(), cursor_before);
}

/// Test TypeScript type alias
#[test]
fn test_typescript_type_alias() {
    let code = r#"type Point = {
    x: number;
    y: number;
};

type ID = string | number;

const point: Point = { x: 10, y: 20 };
const id: ID = 123;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'Point' type usage
    test.keys("7G0");
    test.keys("13l"); // Position on 'Point' in variable declaration

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript enum
#[test]
fn test_typescript_enum() {
    let code = r#"enum Color {
    Red,
    Green,
    Blue
}

const favoriteColor: Color = Color.Blue;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to enum name
    test.keys("0");
    test.keys("5l"); // Position on 'Color'

    test.press('K');
    test.assert_cursor(0, 5);
}

/// Test TypeScript generics
#[test]
fn test_typescript_generics() {
    let code = r#"function identity<T>(arg: T): T {
    return arg;
}

const result = identity<string>("hello");
const numResult = identity(42);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'identity' function call
    test.keys("5G0");
    test.keys("15l"); // Position on 'identity'

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript module imports
#[test]
fn test_typescript_module_imports() {
    let code = r#"import { Component } from 'react';
import * as lodash from 'lodash';
import type { User } from './types';

const MyComponent: Component = () => {
    return null;
};
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'Component' in import
    test.keys("0");
    test.keys("9l"); // Position on 'Component'

    test.keys("gd");
    test.assert_cursor(0, 9); // No movement without LSP
}

/// Test TypeScript async/await
#[test]
fn test_typescript_async_await() {
    let code = r#"async function fetchData(): Promise<string> {
    const response = await fetch('api/data');
    return response.text();
}

async function main() {
    const data = await fetchData();
    console.log(data);
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'fetchData' call
    test.keys("7G0");
    test.keys("23l"); // Position on 'fetchData'

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript decorator syntax
#[test]
fn test_typescript_decorators() {
    let code = r#"function log(target: any, propertyKey: string) {
    console.log(`${propertyKey} was called`);
}

class Calculator {
    @log
    add(a: number, b: number): number {
        return a + b;
    }
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to @log decorator
    test.keys("6G0");
    test.keys("5l"); // Position on 'log' in @log

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript union and intersection types
#[test]
fn test_typescript_union_intersection_types() {
    let code = r#"type StringOrNumber = string | number;
type Employee = { name: string } & { id: number };

const value: StringOrNumber = "hello";
const employee: Employee = { name: "Alice", id: 1 };
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'StringOrNumber' type usage
    test.keys("4G0");
    test.keys("13l");

    test.keys("gd");
    test.assert_cursor(3, 13);
}

/// Test TypeScript namespace
#[test]
fn test_typescript_namespace() {
    let code = r#"namespace Utils {
    export function format(str: string): string {
        return str.toUpperCase();
    }
}

const formatted = Utils.format("hello");
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to namespace name
    test.keys("0");
    test.keys("10l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript optional chaining
#[test]
fn test_typescript_optional_chaining() {
    let code = r#"interface User {
    name?: string;
    address?: {
        city?: string;
    };
}

const user: User = {};
const city = user.address?.city;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to optional chain usage
    test.keys("9G0");
    test.keys("14l"); // Position on 'user'

    test.press('K');
    test.assert_cursor(8, 14);
}

/// Test TypeScript nullish coalescing
#[test]
fn test_typescript_nullish_coalescing() {
    let code = r#"const value: string | null = null;
const defaultValue = value ?? "default";

function getValue(input: number | undefined): number {
    return input ?? 0;
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'value' in nullish coalescing
    test.keys("2G0");
    test.keys("21l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript template literals
#[test]
fn test_typescript_template_literals() {
    let code = r#"const name = "World";
const greeting = `Hello, ${name}!`;

type Greeting = `Hello, ${string}`;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to template literal
    test.keys("2G0");
    test.keys("18l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript as const
#[test]
fn test_typescript_as_const() {
    let code = r#"const colors = ['red', 'green', 'blue'] as const;
type Color = typeof colors[number];

const config = {
    readonly: true,
    version: 1
} as const;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'colors' usage
    test.keys("2G0");
    test.keys("15l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript type guards
#[test]
fn test_typescript_type_guards() {
    let code = r#"function isString(value: unknown): value is string {
    return typeof value === 'string';
}

function process(input: string | number) {
    if (isString(input)) {
        console.log(input.toUpperCase());
    }
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'isString' function call
    test.keys("6G0");
    test.keys("8l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript mapped types
#[test]
fn test_typescript_mapped_types() {
    let code = r#"type User = {
    name: string;
    age: number;
};

type ReadonlyUser = Readonly<User>;
type PartialUser = Partial<User>;
type PickedUser = Pick<User, 'name'>;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'User' in Readonly<User>
    test.keys("6G0");
    test.keys("31l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript conditional types
#[test]
fn test_typescript_conditional_types() {
    let code = r#"type IsString<T> = T extends string ? true : false;
type Result1 = IsString<string>;
type Result2 = IsString<number>;

type NonNullable<T> = T extends null | undefined ? never : T;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'IsString' usage
    test.keys("2G0");
    test.keys("15l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript utility types
#[test]
fn test_typescript_utility_types() {
    let code = r#"interface Todo {
    title: string;
    description: string;
    completed: boolean;
}

type TodoPreview = Pick<Todo, 'title' | 'completed'>;
type TodoReadonly = Readonly<Todo>;
type TodoOptional = Partial<Todo>;
type TodoRequired = Required<TodoOptional>;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'Todo' in Pick
    test.keys("7G0");
    test.keys("25l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript indexed access types
#[test]
fn test_typescript_indexed_access_types() {
    let code = r#"interface Person {
    name: string;
    age: number;
}

type NameType = Person['name'];
type AgeType = Person['age'];
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'Person' in indexed access
    test.keys("6G0");
    test.keys("16l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript JSX/TSX syntax
#[test]
fn test_typescript_jsx() {
    let code = r#"import React from 'react';

interface Props {
    message: string;
}

const Greeting: React.FC<Props> = ({ message }) => {
    return <div>{message}</div>;
};

export default Greeting;
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.tsx"));

    // Move to 'Props' usage
    test.keys("7G0");
    test.keys("25l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript with method chaining
#[test]
fn test_typescript_method_chaining() {
    let code = r#"class StringBuilder {
    private value: string = '';

    append(text: string): this {
        this.value += text;
        return this;
    }

    toString(): string {
        return this.value;
    }
}

const result = new StringBuilder()
    .append("Hello")
    .append(" ")
    .append("World")
    .toString();
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'append' method call
    test.keys("15G0");
    test.keys("5l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript with complex nesting
#[test]
fn test_typescript_complex_nesting() {
    let code = r#"interface Config {
    database: {
        host: string;
        port: number;
        credentials: {
            username: string;
            password: string;
        };
    };
}

const config: Config = {
    database: {
        host: 'localhost',
        port: 5432,
        credentials: {
            username: 'admin',
            password: 'secret'
        }
    }
};
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to 'Config' usage
    test.keys("12G0");
    test.keys("14l");

    test.keys("gd");
    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test file path preserved for TypeScript
#[test]
fn test_typescript_file_path_preservation() {
    let mut test = EditorTest::new("const x = 1;\n");
    test.set_file_path("/workspace/src/index.ts".to_string());

    assert_eq!(
        test.editor.buffer().file_path(),
        Some("/workspace/src/index.ts")
    );
}

/// Test .tsx file extension
#[test]
fn test_tsx_file_extension() {
    let mut test = EditorTest::new("const App = () => <div>Hello</div>;\n");
    test.set_file_path("/workspace/src/App.tsx".to_string());

    assert_eq!(
        test.editor.buffer().file_path(),
        Some("/workspace/src/App.tsx")
    );
}

/// Test gd works after editing TypeScript code
#[test]
fn test_gd_after_typescript_editing() {
    let code = r#"function test() {
    return 42;
}
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Add a call to the function
    test.keys("GA");
    test.type_text("\nconst result = test();");
    test.press_esc();

    // Move to 'test' call
    test.keys("4G0");
    test.keys("15l");

    // Try goto definition
    test.keys("gd");

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test hover after modifying TypeScript code
#[test]
fn test_hover_after_typescript_modification() {
    let mut test = EditorTest::new("let value = 100;\n");
    test.set_file_path(temp_test_path("test.ts"));

    // Modify the value
    test.keys("0");
    test.keys("w"); // Move to 'value'
    test.keys("cw"); // Change word
    test.type_text("newValue");
    test.press_esc();

    // Request hover on the new identifier
    test.press('K');

    test.assert_mode(ovim::mode::Mode::Normal);
}

/// Test TypeScript with comments
#[test]
fn test_typescript_with_comments() {
    let code = r#"/**
 * Adds two numbers
 * @param a First number
 * @param b Second number
 * @returns Sum of a and b
 */
function add(a: number, b: number): number {
    return a + b; // Return the sum
}

// Call the function
const result = add(5, 3);
"#;

    let mut test = EditorTest::new(code);
    test.set_file_path(temp_test_path("test.ts"));

    // Move to function name
    test.keys("7G0");
    test.keys("9l");

    test.press('K');
    test.assert_mode(ovim::mode::Mode::Normal);
}
