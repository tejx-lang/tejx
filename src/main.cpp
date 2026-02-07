#include <iostream>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#include <cstdlib> // For system()

#include "tejx/Lexer.h"
#include "tejx/Parser.h"
#include "tejx/CodeGen.h"

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
    
    // Determine output name
    std::string outputName = "a.out";
    size_t lastDot = filename.find_last_of(".");
    if (lastDot != std::string::npos) {
        outputName = filename.substr(0, lastDot);
        // Strip path directories if present, just simplistic logic for now or keep relative.
        // If examples/hello.tx -> examples/hello
    } else {
        outputName = filename + ".out";
    }

    std::string source = readFile(filename);
    
    // 1. Lexing
    tejx::Lexer lexer(source);
    std::vector<tejx::Token> tokens = lexer.tokenize();

    // 2. Parsing
    tejx::Parser parser(tokens);
    auto program = parser.parse();
    
    // 3. Code Generation
    tejx::CodeGen codegen;
    std::string cppCode = codegen.generate(program);
    std::cerr << "DEBUG: Main generated code length: " << cppCode.length() << std::endl;
    
    // 4. Output Code
    std::string tempFile = outputName + ".cpp";
    std::ofstream outFile(tempFile);
    outFile << cppCode;
    outFile.close();
    
    // 5. Compile with Clang++
    std::string compileCmd = "clang++ -std=c++17 " + tempFile + " -o " + outputName;
    int result = system(compileCmd.c_str());
    
    if (result == 0) {
        std::cerr << "Successfully compiled to '" << outputName << "'" << std::endl;
        // Cleanup intermediate file - Disabled to preserve for build/examples
        // remove(tempFile.c_str()); 
    } else {
        std::cerr << "Compilation failed." << std::endl;
        return 1;
    }

    return 0;
}
