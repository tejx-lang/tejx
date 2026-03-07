#define _USE_MATH_DEFINES
#include <iostream>
#include <vector>
#include <chrono>
#include <iomanip>
#include <cmath>
#include <string>
#include <unordered_map>
#include <algorithm>
#include <random>
#include <numeric>

using namespace std;
using namespace std::chrono;

template <typename T>
void DoNotOptimize(T const& value) {
    asm volatile("" : : "r,m"(value) : "memory");
}

struct Node {
    int value;
    Node* next;
};

struct Vec3 {
    float x;
    float y;
    float z;
};

double bench_sieve() {
    int limit = 50000000;
    vector<bool> primes(limit + 1, true);
    for (int p = 2; p * p <= limit; p++) {
        if (primes[p]) {
            for (int i = p * p; i <= limit; i += p) primes[i] = false;
        }
    }
    double count = 0.0;
    for (int p = 2; p <= limit; p++) if (primes[p]) count += 1.0;
    return count;
}

double bench_pointers() {
    int size = 10000000;
    vector<Node> nodes(size);
    for (int i = 0; i < size; i++) {
        nodes[i].value = i;
        nodes[i].next = nullptr;
    }
    for (int j = 0; j < size - 1; j++) nodes[j].next = &nodes[j + 1];
    
    Node* curr = &nodes[0];
    long long sum = 0;
    while (curr != nullptr) {
        sum += curr->value;
        curr = curr->next;
    }
    return (double)sum;
}

double bench_math() {
    double a = 1.01, b = 1.00, c = 1.03;
    for (int i = 0; i < 500000000; i++) {
        a = a * b + c;
    }
    return (double)a;
}

double bench_matrix() {
    int N = 4096;
    vector<double> A(N * N, 1.0);
    vector<double> B(N * N, 0.0);
    for (int i = 0; i < N; i++) {
        for (int j = 0; j < N; j++) {
            B[j * N + i] = A[i * N + j];
        }
    }
    return B[0];
}

double bench_binary_search() {
    vector<int> data(10000000);
    for (int i = 0; i < 10000000; i++) data[i] = i;
    int found = 0;
    for (int i = 0; i < 10000000; i++) {
        int low = 0, high = 9999999;
        while (low <= high) {
            int mid = low + (high - low) / 2;
            if (data[mid] == i) { found++; break; }
            if (data[mid] < i) low = mid + 1;
            else high = mid - 1;
        }
    }
    return (double)found;
}

double bench_mandelbrot() {
    int count = 0;
    for (int y = 0; y < 2000; y++) {
        for (int x = 0; x < 2000; x++) {
            double cr = x * 0.002 - 1.5, ci = y * 0.002 - 1.0;
            double zr = 0.0, zi = 0.0;
            int k = 0;
            while (zr * zr + zi * zi < 4.0 && k < 200) {
                double tmp = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = tmp;
                k++;
            }
            if (k == 200) count++;
        }
    }
    return (double)count;
}

double bench_branching() {
    vector<int> data(20000000);
    for (int i = 0; i < 20000000; i++) data[i] = i % 100;
    long long sum = 0;
    for (int j = 0; j < 20; j++) {
        for (int i = 0; i < 20000000; i++) {
            if (data[i] < 50) sum += data[i];
        }
    }
    return (double)sum;
}

double bench_vec_norm() {
    int size = 10000000;
    vector<Vec3> vecs(size, { 1.1f, 2.2f, 3.3f });
    for (int i = 0; i < size; i++) {
        Vec3& v = vecs[i];
        float mag = 1.0f / sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
        v.x *= mag; v.y *= mag; v.z *= mag;
    }
    return vecs[0].x;
}

double bench_gamma() {
    double res = 1.0;
    for (int i = 0; i < 20000000; i++) {
        double n = 5.0 + (i % 5);
        res = sqrt(2.0 * M_PI * n) * pow(n / M_E, n);
    }
    return (double)res;
}

double bench_strings() {
    string s = "";
    for (int i = 0; i < 50000000; i++) {
        s += "a";
        if (s.length() > 100) s = ""; 
    }
    return (double)s.length();
}

double bench_bits() {
    vector<int> bits(10000000, 0);
    for (int i = 0; i < 200000000; i++) {
        bits[i % 10000000] ^= (1 << (i % 31));
    }
    return (double)bits[0];
}

double bench_trig() {
    double sum = 0.0;
    for (int i = 0; i < 30000000; i++) {
        sum += sin(i * 0.01) * cos(i * 0.02);
    }
    return (double)sum;
}

double bench_map() {
    unordered_map<int, int> m;
    for (int i = 0; i < 5000000; i++) m[i] = i;
    return (double)m.size();
}

struct PObj {
    int a;
    int b;
};

double bench_heap() {
    double res = 0.0;
    for (int i = 0; i < 10000000; i++) {
        PObj* p = new PObj{i, i * 2};
        if (i == 9999999) res = p->a * 1.0;
        delete p;
    }
    return res;
}

double bench_copy() {
    vector<int> src(100000, 1);
    vector<int> dst(100000, 0);
    for (int i = 0; i < 5000; i++) {
        for (int j = 0; j < 100000; j++) dst[j] = src[j]; // Naive copy to match JS/TX
    }
    return dst[0];
}

double bench_div() {
    int res = 5000000;
    for (int i = 1; i < 200000000; i++) {
        res /= (i % 10 + 1);
        if (res == 0) res = 5000000;
    }
    return (double)res;
}

int fib(int n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}
double bench_fib() {
    return (double)fib(40);
}

double bench_bubble() {
    int first = 0;
    for (int k = 0; k < 1000000; k++) {
        int arr[10] = { 9,8,7,6,5,4,3,2,1,0 };
        for (int i = 0; i < 10; i++) {
            for (int j = 0; j < 9; j++) {
                if (arr[j] > arr[j + 1]) {
                    int t = arr[j]; arr[j] = arr[j + 1]; arr[j + 1] = t;
                }
            }
        }
        if (k == 999999) first = arr[0];
    }
    return (double)first + 0.1;
}

double bench_float_sum() {
    double sum = 0.0;
    for (int i = 0; i < 500000000; i++) sum += 0.00001;
    return (double)sum;
}

struct PropObj {
    int a;
    int b;
    int c;
    int d;
};

double bench_obj_access() {
    PropObj obj = { 1, 2, 3, 4 };
    long long sum = 0;
    for (int i = 0; i < 500000000; i++) {
        sum += obj.a + obj.b + obj.c + obj.d;
    }
    return (double)sum;
}

int solve_nqueens(int n, int row, int col, int diag1, int diag2) {
    if (row == n) return 1;
    int count = 0;
    int available = ((1 << n) - 1) & ~(col | diag1 | diag2);
    while (available != 0) {
        int pos = available & -available;
        available ^= pos;
        count += solve_nqueens(n, row + 1, col | pos, (diag1 | pos) << 1, (diag2 | pos) >> 1);
    }
    return count;
}

double bench_nqueens() {
    return (double)solve_nqueens(14, 0, 0, 0, 0);
}

int ackermann(int m, int n) {
    if (m == 0) return n + 1;
    if (m > 0 && n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

double bench_ackermann() {
    return (double)ackermann(3, 10);
}

double bench_float_to_int() {
    int sum = 0;
    for (int i = 0; i < 200000000; i++) {
        float f = (float)i * 1.5f;
        int v = (int)f;
        sum += v;
        sum ^= v;
    }
    return (double)sum + 0.1;
}

void run(string name, double (*f)()) {
    cout << left << setw(35) << name << flush;
    auto s = high_resolution_clock::now();
    double res = f();
    auto e = high_resolution_clock::now();
    cout << " -> [" << setprecision(2) << res << "] : " << fixed << setprecision(2) << duration<double, milli>(e - s).count() << " ms" << endl;
}

int main() {
    cout << "--- Unified Performance Suite (23 Tests) ---" << endl;
    run("1. Sieve of Eratosthenes", bench_sieve);
    run("2. Pointer Chasing", bench_pointers);
    run("3. Math Throughput", bench_math);
    run("4. Matrix Transpose", bench_matrix);
    run("5. Binary Search", bench_binary_search);
    run("6. Mandelbrot Set", bench_mandelbrot);
    run("7. Branch Prediction", bench_branching);
    run("8. Vector Normalization", bench_vec_norm);
    run("9. Gamma Approx (Math)", bench_gamma);
    run("10. String Operations", bench_strings);
    run("11. Bitwise Logic", bench_bits);
    run("12. Trig Synthesis", bench_trig);
    run("13. Map/Dictionary Load", bench_map);
    run("14. Heap Stress", bench_heap);
    run("15. Array Copy", bench_copy);
    run("16. Division Stress", bench_div);
    run("17. Fibonacci (Recursive)", bench_fib);
    run("18. Small Bubble Sort", bench_bubble);
    run("19. Float Summation", bench_float_sum);
    run("20. Property Access", bench_obj_access);
    run("21. Hard: N-Queens", bench_nqueens);
    run("22. Hard: Ackermann", bench_ackermann);
    run("23. Regress: Float-to-Int", bench_float_to_int);
    return 0;
}
