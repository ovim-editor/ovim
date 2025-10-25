/**
 * This is an example Java file.
 */
public class TestJava {
    private String name;
    private int value;

    public TestJava(String name, int value) {
        this.name = name;
        this.value = value;
    }

    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public int getValue() {
        return value;
    }

    public void setValue(int value) {
        this.value = value;
    }

    @Override
    public String toString() {
        return "TestJava{name='" + name + "', value=" + value + "}";
    }

    public static void main(String[] args) {
        TestJava test = new TestJava("example", 42);
        System.out.println(test.toString());
    }
}
