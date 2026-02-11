#include <iostream>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#include <cstdlib> // For system()
#include <set>
#include <map>
#include <utility>
#include <filesystem>

#include "tejx/Lexer.h"
#include "tejx/Parser.h"
#include "tejx/AST.h"
#include "tejx/Lowering.h"
#include "tejx/TypeChecker.h"
#include "tejx/MIRLowering.h"
#include "tejx/BorrowChecker.h"
#include "tejx/MIRCodeGen.h"

namespace fs = std::filesystem;

void printUsage() {
    std::cout << "Usage: tejxc <filename.tx>" << std::endl;
}

std::string readFile(const std::string& path) {
    std::ifstream file(path);
    if (!file.is_open()) {
        std::cerr << "Could not open file: " << path << std::endl;
        exit(1);
    }
    std::stringstream buffer;
    buffer << file.rdbuf();
    return buffer.str();
}

int main(int argc, char* argv[]) {
    if (argc < 2) {
        printUsage();
        return 1;
    }

    std::string filename = argv[1];
    std::string source = readFile(filename);
    
    // 1. Lexing
    // std::cout << "[1] Lexing..." << std::endl;
    tejx::Lexer lexer(source);
    std::vector<tejx::Token> tokens = lexer.tokenize();

    // 2. Parsing
    // std::cout << "[2] Parsing..." << std::endl;
    tejx::Parser parser(tokens);
    auto program = parser.parse();
    
    if (!parser.errors.empty()) {
        for (const auto& err : parser.errors) {
            std::cerr << err << std::endl;
        }
        return 1;
    }

    // 3. Lowering to HIR
    // std::cout << "[3] Lowering to HIR..." << std::endl;
    tejx::Lowering lowering;
    auto hir = lowering.lower(program);

    // 4. Semantic Analysis
    // std::cout << "[4] Semantic Analysis..." << std::endl;
    tejx::TypeChecker typeChecker;
    typeChecker.check(hir);
    if (!typeChecker.errors.empty()) {
        for (const auto& err : typeChecker.errors) {
            std::cerr << err << std::endl;
        }
        return 1;
    }

    // 5. Lowering to MIR
    // std::cout << "[5] Lowering to MIR..." << std::endl;
    tejx::MIRLowering mirLowering;
    auto mir = mirLowering.lower(hir);

    // 6. Borrow Checking
    // std::cout << "[6] Borrow Checking..." << std::endl;
    tejx::BorrowChecker borrowChecker;
    borrowChecker.check(mir);
    if (!borrowChecker.errors.empty()) {
        std::cerr << "Borrow Checker Found Errors!" << std::endl;
        return 1;
    }
    
    // 7. LLVM IR Generation
    // std::cout << "[7] LLVM IR Generation..." << std::endl;
    tejx::MIRCodeGen codeGen;
    std::vector<std::shared_ptr<tejx::MIRFunction>> funcs;
    funcs.push_back(mir);
    std::string llvmCode = codeGen.generate(funcs);
    
    // Determine output name
    std::string outputName = "a.out";
    std::string baseName = filename;
    size_t lastDot = filename.find_last_of(".");
    if (lastDot != std::string::npos) {
        baseName = filename.substr(0, lastDot);
        outputName = baseName;
    }

    std::string tempFile = baseName + ".ll";
    std::ofstream outFile(tempFile);
    outFile << llvmCode;
    outFile.close();
    
    // 8. Optimization
    // std::cout << "[8] Optimization (LLVM Passes)..." << std::endl;

    // 9. Backend Compile & Link (Clang)
    // Helper to find runtime.c (assuming absolute path for now or relative to current dir)
    std::string runtimePath = "/Users/praveenyadav/Desktop/My Projects/NovaJs/src/runtime/runtime.c"; 
    
    // std::cout << "[9] Machine Code Generation & Linking..." << std::endl;
    std::string compileCmd = "clang++ -O3 -Wno-deprecated -Wno-override-module " + tempFile + " \"" + runtimePath + "\" -o " + outputName;
    // std::cout << "Compiling: " << compileCmd << std::endl;
    int result = system(compileCmd.c_str());
    
    if (result == 0) {
        // std::cout << "Build successful! Binary: " << outputName << std::endl;
        remove(tempFile.c_str()); 
    } else {
        std::cerr << "Compilation failed." << std::endl;
        return 1;
    }

    return 0;
}
