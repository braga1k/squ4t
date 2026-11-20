// Records what 1nstall's ElasticMotion (src/motion.cs, unmodified) computes, for the Rust port to
// be compared against. Built by oracle/motion/record.ps1 with the C# compiler that ships with Windows.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using OneInstall;

static class PoseDump {
    static string N(double v) { return v.ToString("R", CultureInfo.InvariantCulture); }
    static void Main(string[] args) {
        var feels = new Dictionary<string, ElasticFeel> { { "balanced", ElasticFeel.Balanced }, { "firm", ElasticFeel.Firm }, { "elastic", ElasticFeel.Elastic } };
        var froms = new List<ElasticMotion.Pose> {
            new ElasticMotion.Pose(0, 0, 1), new ElasticMotion.Pose(16, 0, 1),
            new ElasticMotion.Pose(9, 4, 0.95), new ElasticMotion.Pose(3, 7.5, 0.9), new ElasticMotion.Pose(14.2, 1.3, 1.02),
            new ElasticMotion.Pose(1.66, 0, 0.9502), new ElasticMotion.Pose(14.34, 0, 0.9502), new ElasticMotion.Pose(-1.5, 2.8, 1.05),
        };
        var sb = new StringBuilder();
        sb.Append("{\"spring\": [");
        bool first = true;
        foreach (double bounce in new[] { 0, 0.3, 0.8 })
            for (int i = 0; i <= 40; i++) {
                if (!first) sb.Append(", "); first = false;
                sb.Append("[" + N(bounce) + ", " + N(i / 40.0) + ", " + N(ElasticMotion.Spring(i / 40.0, bounce)) + "]");
            }
        sb.Append("],\n\"flights\": [");
        first = true;
        foreach (var feel in feels)
            foreach (var from in froms)
                foreach (double target in new double[] { 0, 16 }) {
                    if (!first) sb.Append(",\n"); first = false;
                    var poses = ElasticMotion.Flight(from, target, feel.Value);
                    sb.Append("{\"feel\": \"" + feel.Key + "\", \"from\": [" + N(from.X) + ", " + N(from.Stretch) + ", " + N(from.Height) + "], \"target\": " + N(target) + ", \"poses\": [");
                    for (int i = 0; i < poses.Length; i++) {
                        if (i > 0) sb.Append(", ");
                        sb.Append("[" + N(poses[i].X) + ", " + N(poses[i].Stretch) + ", " + N(poses[i].Height) + "]");
                    }
                    sb.Append("]}");
                }
        sb.Append("]}\n");
        File.WriteAllText(args[0], sb.ToString(), new UTF8Encoding(false));
    }
}
