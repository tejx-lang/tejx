const { performance } = require('perf_hooks');

let sink = 0;
function DoNotOptimize(val) {
    if (typeof val === 'number') sink += val;
}

function bench_sieve() {
    let limit = 50000000;
    let primes = new Uint8Array(limit + 1).fill(1);
    for (let p = 2; p * p <= limit; p++) {
        if (primes[p]) {
            for (let i = p * p; i <= limit; i += p) primes[i] = 0;
        }
    }
    let count = 0.0;
    for (let p = 2; p <= limit; p++) if (primes[p]) count += 1.0;
    return count;
}

function bench_pointers() {
    let size = 10000000;
    let nodes = new Array(size);
    for (let i = 0; i < size; i++) nodes[i] = { value: i, next: null };
    for (let j = 0; j < size - 1; j++) nodes[j].next = nodes[j + 1];

    let curr = nodes[0];
    let sum = 0;
    while (curr !== null) {
        sum += curr.value;
        curr = curr.next;
    }
    return sum;
}

function bench_math() {
    let a = 1.01, b = 1.00, c = 1.03;
    for (let i = 0; i < 500000000; i++) {
        a = a * b + c;
    }
    return a;
}

function bench_matrix() {
    let N = 4096;
    let A = new Float64Array(N * N);
    let B = new Float64Array(N * N);
    A.fill(1.0);
    B.fill(0.0);
    for (let i = 0; i < N; i++) {
        for (let j = 0; j < N; j++) {
            B[j * N + i] = A[i * N + j];
        }
    }
    return B[0];
}

function bench_binary_search() {
    let data = new Int32Array(10000000);
    for (let i = 0; i < 10000000; i++) data[i] = i;
    let found = 0;
    for (let i = 0; i < 10000000; i++) {
        let low = 0, high = 9999999;
        while (low <= high) {
            let mid = (low + high) >> 1;
            if (data[mid] === i) { found++; break; }
            if (data[mid] < i) low = mid + 1;
            else high = mid - 1;
        }
    }
    return found;
}

function bench_mandelbrot() {
    let count = 0;
    for (let y = 0; y < 2000; y++) {
        for (let x = 0; x < 2000; x++) {
            let cr = x * 0.002 - 1.5, ci = y * 0.002 - 1.0;
            let zr = 0.0, zi = 0.0, k = 0;
            while (zr * zr + zi * zi < 4.0 && k < 200) {
                let tmp = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = tmp;
                k++;
            }
            if (k === 200) count++;
        }
    }
    return count;
}

function bench_branching() {
    let data = new Int32Array(20000000);
    for (let i = 0; i < 20000000; i++) data[i] = i % 100;
    let sum = 0;
    for (let j = 0; j < 20; j++) {
        for (let i = 0; i < 20000000; i++) {
            if (data[i] < 50) sum += data[i];
        }
    }
    return sum;
}

function bench_vec_norm() {
    let size = 10000000;
    let vecs = new Array(size);
    for (let i = 0; i < size; i++) vecs[i] = { x: 1.1, y: 2.2, z: 3.3 };
    for (let i = 0; i < size; i++) {
        let v = vecs[i];
        let mag = 1.0 / Math.sqrt(v.x * v.x + v.y * v.y + v.z * v.z);
        v.x *= mag; v.y *= mag; v.z *= mag;
    }
    return vecs[0].x;
}

function bench_gamma() {
    let res = 1.0;
    for (let i = 0; i < 20000000; i++) {
        let n = 5.0 + (i % 5);
        res = Math.sqrt(2.0 * Math.PI * n) * Math.pow(n / Math.E, n);
    }
    return res;
}

function bench_strings() {
    let s = "";
    for (let i = 0; i < 50000000; i++) {
        s += "a";
        if (s.length > 100) s = "";
    }
    return s.length;
}

function bench_bits() {
    let bits = new Int32Array(10000000);
    for (let i = 0; i < 200000000; i++) {
        bits[i % 10000000] ^= (1 << (i % 31));
    }
    return bits[0];
}

function bench_trig() {
    let sum = 0.0;
    for (let i = 0; i < 30000000; i++) {
        sum += Math.sin(i * 0.01) * Math.cos(i * 0.02);
    }
    return sum;
}

function bench_map() {
    let m = new Map();
    for (let i = 0; i < 5000000; i++) m.set(i, i);
    return m.size;
}

function bench_heap() {
    let p_a = 0;
    for (let i = 0; i < 10000000; i++) {
        let p = { a: i, b: i * 2 };
        p_a = p.a;
    }
    return p_a * 1.0;
}

function bench_copy() {
    let src = new Int32Array(100000).fill(1);
    let dst = new Int32Array(100000).fill(0);
    for (let i = 0; i < 5000; i++) {
        for (let j = 0; j < 100000; j++) dst[j] = src[j]; // Naive copy
    }
    return dst[0];
    return dst[0];
}

function bench_div() {
    let res = 5000000;
    for (let i = 1; i < 200000000; i++) {
        res = Math.trunc(res / ((i % 10) + 1));
        if (res === 0) res = 5000000;
    }
    return res * 1.0;
}

function fib(n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}
function bench_fib() {
    return fib(40);
}

function bench_bubble() {
    let first = 0;
    for (let k = 0; k < 1000000; k++) {
        let arr = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0];
        for (let i = 0; i < 10; i++) {
            for (let j = 0; j < 9; j++) {
                if (arr[j] > arr[j + 1]) {
                    let t = arr[j]; arr[j] = arr[j + 1]; arr[j + 1] = t;
                }
            }
        }
        if (k === 999999) first = arr[0];
    }
    return first * 1.0 + 0.1;
}

function bench_float_sum() {
    let sum = 0.0;
    for (let i = 0; i < 500000000; i++) sum += 0.00001;
    return sum;
}

function bench_obj_access() {
    let obj = { a: 1, b: 2, c: 3, d: 4 };
    let sum = 0;
    for (let i = 0; i < 500000000; i++) {
        sum += obj.a + obj.b + obj.c + obj.d;
    }
    return sum;
}

function solve_nqueens(n, row, col, diag1, diag2) {
    if (row === n) return 1;
    let count = 0;
    let available = ((1 << n) - 1) & ~(col | diag1 | diag2);
    while (available !== 0) {
        let pos = available & -available;
        available ^= pos;
        count += solve_nqueens(n, row + 1, col | pos, (diag1 | pos) << 1, (diag2 | pos) >>> 1);
    }
    return count;
}
function bench_nqueens() {
    return solve_nqueens(14, 0, 0, 0, 0);
}

function ackermann(m, n) {
    if (m === 0) return n + 1;
    if (m > 0 && n === 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}
function bench_ackermann() {
    return ackermann(3, 10);
}

function bench_float_to_int() {
    let sum = 0;
    for (let i = 0; i < 20000000; i++) {
        let f = i * 1.5;
        let v = f | 0; // Truncate to int like fptosi
        sum += v;
        sum ^= v;
    }
    return sum * 1.0 + 0.1;
}

const tests = {
    "1. Sieve of Eratosthenes": bench_sieve,
    "2. Pointer Chasing": bench_pointers,
    "3. Math Throughput": bench_math,
    "4. Matrix Transpose": bench_matrix,
    "5. Binary Search": bench_binary_search,
    "6. Mandelbrot Set": bench_mandelbrot,
    "7. Branch Prediction": bench_branching,
    "8. Vector Normalization": bench_vec_norm,
    "9. Gamma Approx (Math)": bench_gamma,
    "10. String Operations": bench_strings,
    "11. Bitwise Logic": bench_bits,
    "12. Trig Synthesis": bench_trig,
    "13. Map/Dictionary Load": bench_map,
    "14. Heap Stress": bench_heap,
    "15. Array Copy": bench_copy,
    "16. Division Stress": bench_div,
    "17. Fibonacci (Recursive)": bench_fib,
    "18. Small Bubble Sort": bench_bubble,
    "19. Float Summation": bench_float_sum,
    "20. Property Access": bench_obj_access,
    "21. Hard: N-Queens": bench_nqueens,
    "22. Hard: Ackermann": bench_ackermann,
    "23. Regress: Float-to-Int": bench_float_to_int
};

function run() {
    console.log("--- Unified Performance Suite (23 Tests) ---");
    for (const [name, fn] of Object.entries(tests)) {
        let start = performance.now();
        let res = fn();
        let end = performance.now();
        console.log(`${name.padEnd(30)} -> [${res}] : ${(end - start).toFixed(2)} ms`);
    }
}

run();
