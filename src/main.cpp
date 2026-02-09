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
#include "tejx/CodeGen.h"
#include "tejx/AST.h"

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

// Module System State
std::set<std::string> visitedModules;
std::vector<std::pair<std::string, std::shared_ptr<tejx::Program>>> modules; // Ordered list of modules

void compileModule(const std::string& path) {
    std::string canonicalPath;
    try {
        if (!fs::exists(path)) {
            std::cerr << "Error: Module not found: " << path << std::endl;
            exit(1);
        }
        canonicalPath = fs::canonical(path).string();
    } catch (const std::exception& e) {
        std::cerr << "Error resolving path " << path << ": " << e.what() << std::endl;
        exit(1);
    }

    if (visitedModules.count(canonicalPath)) return; // Already processed
    visitedModules.insert(canonicalPath);

    std::string source = readFile(canonicalPath);
    
    // 1. Lexing
    tejx::Lexer lexer(source);
    std::vector<tejx::Token> tokens = lexer.tokenize();

    // 2. Parsing
    tejx::Parser parser(tokens);
    auto program = parser.parse();
    
    // 3. Recursive Import Scanning
    std::string baseDir = fs::path(path).parent_path().string();
    if (baseDir.empty()) baseDir = ".";

    for (const auto& stmt : program->statements) {
        if (auto importDecl = std::dynamic_pointer_cast<tejx::ImportDecl>(stmt)) {
            std::string importPath = importDecl->source;
            // Handle relative paths
            if (importPath.find("./") == 0 || importPath.find("../") == 0) {
                 importPath = baseDir + "/" + importPath;
            }
            if (importPath.find(".tx") == std::string::npos) {
                importPath += ".tx";
            }
            compileModule(importPath);
        }
    }
    
    // Add to modules list (post-order traversal ensures imports are compiled before importers)
    // Actually, for C++ generation w/ forward decls, order matters less for types, but matters for 'using'
    // Let's store in the order we finished parsing them (dependencies first)
    modules.push_back({canonicalPath, program});
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
    } else {
        outputName = filename + ".out";
    }

    // Start recursive compilation
    compileModule(filename);
    
    // 3. Code Generation (Pass all modules)
    tejx::CodeGen codegen;
    std::string cppCode = codegen.generate(modules);
    
    // 4. Output Code
    std::string tempFile = outputName + ".cpp";
    std::ofstream outFile(tempFile);
    outFile << cppCode;
    outFile.close();
    
    // 5. Compile with Clang++
    std::string compileCmd = "clang++ -std=c++17 " + tempFile + " -o " + outputName;
    int result = system(compileCmd.c_str());
    
    if (result == 0) {
        std::cerr << "Compiler Build successful! Binary located at: " << outputName << std::endl;
        // remove(tempFile.c_str()); 
    } else {
        std::cerr << "Compilation failed." << std::endl;
        return 1;
    }

    return 0;
}
