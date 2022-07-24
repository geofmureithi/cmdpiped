const os = require('os');

const readline = require('readline');

function askQuestion(query) {
    const rl = readline.createInterface({
        input: process.stdin,
        output: process.stdout,
    });

    return new Promise(resolve => rl.question(query, ans => {
        rl.close();
        resolve(ans);
    }))
}
async function main() {
    const ans = await askQuestion("What would you want to know about me?")
    console.log(ans)
    if (ans == "cpu") console.log(JSON.stringify(os.cpus()))
    if (ans == "totalmem") console.log(os.totalmem())
    if (ans == "freemem") console.log(os.freemem())
}

main()
