package dev.kampr.terminal

import dev.kampr.terminal.guard.destructiveLine
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The false-positive corpus is the half that decides whether the feature survives a real user: an
// over-eager confirm on a phone teaches the thumb to tap through, and then it protects nothing.
private val SAFE = listOf(
    "rm build/tmp.o",
    "rm -f build/tmp.o",
    "rm *.o",
    "ls -la",
    "cd ~ && ls",
    "git push origin main",
    "git push --force-with-lease origin feat/x",
    "git status",
    "git reset HEAD~1",
    "git clean -n",
    "git commit -m \"drop the -rf handling\"",
    "git log --oneline | head -20",
    "echo \"rm -rf /\"",
    "echo rm -rf /",
    "man sudo",
    "which sudo",
    "grep -r sudo /etc",
    "history | grep 'rm -rf'",
    "cat notes.md | grep DROP",
    "sed -i 's/DROP TABLE/keep/' schema.sql",
    "printf 'DROP TABLE %s\\n' users",
    "vim /etc/sudoers",
    "alias rm='rm -i'",
    "sudoku --hard",
    "ddgr kotlin multiplatform",
    "pgrep dd",
    "mkfifo /tmp/pipe",
    "cargo build --release",
    "npm run build",
    "./gradlew build",
    "echo hi > out.log",
    "echo hi >> /var/log/mine.log",
    "cat /dev/urandom | head -c 10 > sample.bin",
    "curl -s https://herdr.dev > /dev/null",
    "make 2>&1 | tee build.log",
    "docker ps -a",
    "docker run --rm alpine echo hi",
    "kubectl get pods -A",
    "kubectl delete --dry-run=client -f deploy.yaml",
    "chmod 777 script.sh",
    "chmod -R u+rw src",
    "truncated_report.py --all",
    "dd if=/dev/sda | gzip > backup.gz",
    "ssh nas 'rm -rf /tmp/scratch'",
    "find . -name '*.tmp' -delete",
    "# rm -rf /",
    "rg 'rm -rf' --glob '!node_modules'",
    "git config --global alias.rmrf '!rm -rf'",
    "grep -rn 'kubectl delete' .",
    "kubectl delete --help",
    "rm -ri old/",
    "rm -rf",
    "truncate",
    "sudo",
    "SUDO_ASKPASS=/usr/bin/x ls",
    "echo \$SUDO_USER",
    "chmod -R 755 /var/www",
    "chown -R me:me src",
    "cp -r src dst",
    "tar -xzf pkg.tgz -C /usr/local",
    "psql -c 'select * from drop_log'",
    "mysqldump prod > prod.sql",
    "tee /etc/hosts",
    "vim +/DROP\\ TABLE schema.sql",
    "python -c \"import os; os.remove('x')\"",
    "npm ci && npm run test -- --force",
    "docker build --no-cache -t x .",
    "brew cleanup --prune=all",
    "nix-collect-garbage -d",
    "> log.txt",
    "mkfs.ext4 --help",
    "",
    "   ",
)

private val DANGEROUS = listOf(
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf \$HOME/",
    "rm -rf /usr/local",
    "rm -rf build",
    "rm -r node_modules",
    "rm --recursive --force dist",
    "/bin/rm -rf build",
    "time rm -rf build",
    "sudo rm -rf /var/lib/thing",
    "sudo apt install ripgrep",
    "sudo -v",
    "dd if=~/arch.iso of=/dev/sdb bs=4M",
    "mkfs.ext4 /dev/sda1",
    "cat image.img > /dev/sda",
    "echo comingclean > /etc/hostname",
    "git push --force origin main",
    "git push -f",
    "sudo git push --force",
    "git reset --hard HEAD~3",
    "git clean -fd",
    "git clean -fdx",
    "psql -c \"DROP TABLE users\"",
    "mysql -e 'DROP DATABASE prod'",
    "DROP TABLE sessions;",
    "TRUNCATE TABLE audit;",
    "truncate -s 0 app.log",
    "shred -u secrets.env",
    "chmod -R 777 /var/www",
    "docker system prune -af",
    "docker volume prune",
    "kubectl delete pod api-7f9",
    "kubectl delete -f deploy.yaml",
    "ls && rm -rf /",
    "make build; sudo make install",
    "for f in *; do rm -rf \$f; done",
    "if [ -d build ]; then rm -rf build; fi",
    "ls | xargs rm -rf",
    "docker image prune -f",
    "cat > /etc/nginx/nginx.conf",
    "> /etc/motd",
    "dd of=out.img if=/dev/zero bs=1M count=10",
)

class DestructiveTest {
    @Test
    fun theFalsePositiveCorpusStaysSilent() {
        val fired = SAFE.mapNotNull { line -> destructiveLine(line)?.let { "$line -> ${it.reason}" } }
        assertTrue(fired.isEmpty(), "these must not interrupt:\n" + fired.joinToString("\n"))
    }

    @Test
    fun everyReviewedPatternFires() {
        val missed = DANGEROUS.filter { destructiveLine(it) == null }
        assertTrue(missed.isEmpty(), "these must interrupt:\n" + missed.joinToString("\n"))
    }

    @Test
    fun theReasonNamesTheWorstThingNotTheFirstThing() {
        assertTrue(destructiveLine("sudo rm -rf /")!!.reason.contains("everything"))
        assertTrue(destructiveLine("sudo apt update")!!.reason.contains("root"))
    }

    @Test
    fun theMatchedCommandIsWhatIsShown() {
        val hit = assertNotNull(destructiveLine("[13:44 dbrain@comingclean ~/dev/kampr]\$ rm -rf build"))
        kotlin.test.assertEquals("rm -rf build", hit.command)
    }

    @Test
    fun aPromptIsStrippedWhateverShellWroteIt() {
        val prompts = listOf(
            "[13:44 dbrain@comingclean ~/dev/kampr]\$ rm -rf build",
            "dbrain@comingclean ~/dev/kampr \$ rm -rf build",
            "root@nas:/srv# rm -rf build",
            "➜  kampr git:(main) ✗ rm -rf build",
            "❯ rm -rf build",
            "kampr on  main [!] via  v1.9.24 ❯ rm -rf build",
            "%  rm -rf build",
            ">   rm -rf build",
        )
        for (prompt in prompts) {
            val hit = destructiveLine(prompt)
            assertNotNull(hit, "no match through: $prompt")
            kotlin.test.assertEquals("rm -rf build", hit.command, "wrong split: $prompt")
        }
    }

    @Test
    fun aRedirectIsNotMistakenForAPrompt() {
        assertNotNull(destructiveLine("\$ rm -rf build > out.log"))
        assertNull(destructiveLine("\$ ls > sudo-notes.txt"))
    }

    @Test
    fun aCommentOnTheEndDoesNotHideTheCommand() {
        assertNotNull(destructiveLine("\$ rm -rf build # frees 2GB"))
    }
}
