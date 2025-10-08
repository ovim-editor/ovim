package com.example;

public class HelloWorld {
    private String message;

    public HelloWorld(String message) {
        this.message = message;
    }

    public String getMessage() {
        return message;
    }

    public void printMessage() {
        System.out.println(message);
    }

    public static void main(String[] args) {
        HelloWorld hello = new HelloWorld("Hello, World!");
        hello.printMessage();
    }
}
